//! MQTT publisher for detections and first-of-day events.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::Serialize;
use sitta_api::event::DetectionEvent;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::MqttConfig;

/// Sanitize a scientific name for use in MQTT topic paths.
/// Lowercase, spaces → underscores, strip non-alphanumeric/underscore.
fn topic_name(scientific_name: &str) -> String {
    scientific_name
        .to_lowercase()
        .replace(' ', "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Tracks which species have been seen today (in the station's timezone).
struct FirstOfDayTracker {
    tz: Tz,
    current_date: NaiveDate,
    seen: HashSet<String>,
}

impl FirstOfDayTracker {
    fn new(timezone: &str) -> Self {
        let tz: Tz = timezone.parse().unwrap_or(Tz::UTC);
        let today = Utc::now().with_timezone(&tz).date_naive();
        Self {
            tz,
            current_date: today,
            seen: HashSet::new(),
        }
    }

    /// Returns true if this is the first detection of the species today.
    /// Resets automatically at midnight in the configured timezone.
    fn is_first_today(&mut self, scientific_name: &str) -> bool {
        let today = Utc::now().with_timezone(&self.tz).date_naive();
        if today != self.current_date {
            self.seen.clear();
            self.current_date = today;
        }
        self.seen.insert(scientific_name.to_string())
    }

    fn species_count(&self) -> usize {
        self.seen.len()
    }

    fn date_string(&self) -> String {
        self.current_date.to_string()
    }
}

#[derive(Serialize)]
struct FirstOfDayPayload {
    scientific_name: String,
    common_name: String,
    taxon_code: Option<String>,
    confidence: f32,
    detected_at: String,
    detection_id: String,
    day: String,
}

#[derive(Serialize)]
struct SpeciesCountPayload {
    count: usize,
    day: String,
}

#[derive(Serialize)]
struct StatusPayload {
    state: &'static str,
}

/// Spawn the MQTT publisher as two background tasks.
pub fn spawn_mqtt_publisher(
    config: &MqttConfig,
    station_id: &str,
    station_name: &str,
    timezone: &str,
    detection_tx: &broadcast::Sender<DetectionEvent>,
    display_min_confidence: f32,
    shutdown: CancellationToken,
) {
    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| format!("sitta-{station_id}"));

    let mut opts = MqttOptions::new(&client_id, &config.host, config.port);
    opts.set_keep_alive(Duration::from_secs(30));

    if let (Some(user), Some(pass)) = (&config.username, &config.password) {
        opts.set_credentials(user, pass);
    }

    // Last Will and Testament — published by broker if we disconnect ungracefully.
    let status_topic = format!("sitta/{station_id}/status");
    let lwt_payload = serde_json::to_string(&StatusPayload { state: "offline" }).unwrap();
    opts.set_last_will(rumqttc::LastWill::new(
        &status_topic,
        lwt_payload,
        QoS::AtLeastOnce,
        true,
    ));

    let (client, mut eventloop) = AsyncClient::new(opts, 128);

    // Shared state for the publisher.
    let sid = station_id.to_string();
    let sname = station_name.to_string();
    let ha_enabled = config.homeassistant_discovery;
    let ha_prefix = config.homeassistant_prefix.clone();
    let first_of_day_conf = config.first_of_day_min_confidence;
    let min_conf = display_min_confidence;

    // Task 1: Drive the MQTT event loop and handle connect/reconnect.
    let client_for_loop = client.clone();
    let sid_for_loop = sid.clone();
    let sname_for_loop = sname.clone();
    let ha_prefix_for_loop = ha_prefix.clone();
    let shutdown_loop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = eventloop.poll() => {
                    match event {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                            tracing::info!("MQTT connected");
                            // Publish online status.
                            let topic = format!("sitta/{}/status", sid_for_loop);
                            let payload = serde_json::to_string(&StatusPayload { state: "online" }).unwrap();
                            let _ = client_for_loop.publish(&topic, QoS::AtLeastOnce, true, payload).await;

                            // Publish HA discovery if enabled.
                            if ha_enabled {
                                publish_ha_discovery(&client_for_loop, &ha_prefix_for_loop, &sid_for_loop, &sname_for_loop).await;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::debug!(error = %e, "MQTT event loop error (will reconnect)");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
                () = shutdown_loop.cancelled() => {
                    // Try to publish offline status before exiting.
                    let topic = format!("sitta/{}/status", sid_for_loop);
                    let payload = serde_json::to_string(&StatusPayload { state: "offline" }).unwrap();
                    let _ = client_for_loop.publish(&topic, QoS::AtLeastOnce, true, payload).await;
                    let _ = client_for_loop.disconnect().await;
                    break;
                }
            }
        }
    });

    // Task 2: Subscribe to detection events and publish to MQTT.
    let mut rx = detection_tx.subscribe();
    let tz_owned = timezone.to_string();
    tokio::spawn(async move {
        let mut tracker = FirstOfDayTracker::new(&tz_owned);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(event) => {
                            if event.confidence < min_conf {
                                continue;
                            }

                            // Publish detection.
                            let det_topic = format!("sitta/{sid}/detection");
                            if let Ok(payload) = serde_json::to_string(&event) {
                                let _ = client.publish(&det_topic, QoS::AtMostOnce, false, payload).await;
                            }

                            // First-of-day check.
                            if event.confidence >= first_of_day_conf
                                && tracker.is_first_today(&event.species.scientific_name)
                            {
                                let sci_topic = topic_name(&event.species.scientific_name);
                                let fod = FirstOfDayPayload {
                                    scientific_name: event.species.scientific_name.clone(),
                                    common_name: event.species.common_name.clone(),
                                    taxon_code: event.species.taxon_code.clone(),
                                    confidence: event.confidence,
                                    detected_at: event.detected_at.clone(),
                                    detection_id: event.id.clone(),
                                    day: tracker.date_string(),
                                };
                                if let Ok(payload) = serde_json::to_string(&fod) {
                                    // Per-species topic (retained).
                                    let _ = client.publish(
                                        format!("sitta/{sid}/first_today/{sci_topic}"),
                                        QoS::AtLeastOnce, true, payload.clone(),
                                    ).await;
                                    // Aggregated topic (retained, for HA sensor).
                                    let _ = client.publish(
                                        format!("sitta/{sid}/first_today"),
                                        QoS::AtLeastOnce, true, payload,
                                    ).await;
                                }

                                // Update species count.
                                let count = SpeciesCountPayload {
                                    count: tracker.species_count(),
                                    day: tracker.date_string(),
                                };
                                if let Ok(payload) = serde_json::to_string(&count) {
                                    let _ = client.publish(
                                        format!("sitta/{sid}/species_today"),
                                        QoS::AtLeastOnce, true, payload,
                                    ).await;
                                }

                                tracing::info!(
                                    species = %event.species.common_name,
                                    confidence = format_args!("{:.2}", event.confidence),
                                    species_today = tracker.species_count(),
                                    "First detection of the day"
                                );
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(dropped = n, "MQTT publisher lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                () = shutdown.cancelled() => break,
            }
        }
    });
}

async fn publish_ha_discovery(client: &AsyncClient, prefix: &str, station_id: &str, station_name: &str) {
    let device = serde_json::json!({
        "identifiers": [format!("sitta_{station_id}")],
        "name": format!("Sitta - {station_name}"),
        "manufacturer": "Sitta",
        "model": "Bioacoustics Engine",
        "sw_version": "0.1.0"
    });
    let avail = serde_json::json!({
        "topic": format!("sitta/{station_id}/status"),
        "value_template": "{{ value_json.state }}",
        "payload_available": "online",
        "payload_not_available": "offline"
    });

    // Station status (binary sensor).
    let _ = client.publish(
        format!("{prefix}/binary_sensor/sitta_{station_id}/status/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_string(&serde_json::json!({
            "name": "Station Status",
            "unique_id": format!("sitta_{station_id}_status"),
            "device": device,
            "state_topic": format!("sitta/{station_id}/status"),
            "value_template": "{{ value_json.state }}",
            "payload_on": "online",
            "payload_off": "offline",
            "device_class": "connectivity",
            "availability": [avail],
        })).unwrap(),
    ).await;

    // Latest detection (sensor).
    let _ = client.publish(
        format!("{prefix}/sensor/sitta_{station_id}/latest_detection/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_string(&serde_json::json!({
            "name": "Latest Detection",
            "unique_id": format!("sitta_{station_id}_latest_detection"),
            "device": device,
            "state_topic": format!("sitta/{station_id}/detection"),
            "value_template": "{{ value_json.species.common_name }}",
            "json_attributes_topic": format!("sitta/{station_id}/detection"),
            "icon": "mdi:bird",
            "availability": [avail],
        })).unwrap(),
    ).await;

    // First bird of day (sensor).
    let _ = client.publish(
        format!("{prefix}/sensor/sitta_{station_id}/first_bird_today/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_string(&serde_json::json!({
            "name": "First Bird of Day",
            "unique_id": format!("sitta_{station_id}_first_bird_today"),
            "device": device,
            "state_topic": format!("sitta/{station_id}/first_today"),
            "value_template": "{{ value_json.common_name }}",
            "json_attributes_topic": format!("sitta/{station_id}/first_today"),
            "icon": "mdi:bird",
            "availability": [avail],
        })).unwrap(),
    ).await;

    // Species count today (sensor).
    let _ = client.publish(
        format!("{prefix}/sensor/sitta_{station_id}/species_today/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_string(&serde_json::json!({
            "name": "Species Detected Today",
            "unique_id": format!("sitta_{station_id}_species_today"),
            "device": device,
            "state_topic": format!("sitta/{station_id}/species_today"),
            "value_template": "{{ value_json.count }}",
            "unit_of_measurement": "species",
            "icon": "mdi:format-list-numbered",
            "availability": [avail],
        })).unwrap(),
    ).await;

    tracing::info!("Published HA MQTT discovery messages");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_name_sanitization() {
        assert_eq!(topic_name("Turdus migratorius"), "turdus_migratorius");
        assert_eq!(topic_name("Tyto alba"), "tyto_alba");
        assert_eq!(topic_name("Passer (domesticus)"), "passer_domesticus");
    }

    #[test]
    fn first_of_day_tracks_species() {
        let mut tracker = FirstOfDayTracker::new("UTC");
        assert!(tracker.is_first_today("Tyto alba"));
        assert!(!tracker.is_first_today("Tyto alba")); // already seen
        assert!(tracker.is_first_today("Strix aluco")); // different species
        assert_eq!(tracker.species_count(), 2);
    }

    #[test]
    fn first_of_day_resets_on_new_date() {
        let mut tracker = FirstOfDayTracker::new("UTC");
        tracker.is_first_today("Tyto alba");
        // Simulate date change by manually setting to yesterday.
        tracker.current_date = tracker.current_date.pred_opt().unwrap();
        assert!(tracker.is_first_today("Tyto alba")); // should be first again
        assert_eq!(tracker.species_count(), 1);
    }
}
