# MQTT Integration Plan

Publish detections and first-of-day events to an MQTT broker for Home
Assistant integration. Config-gated (no feature flag — absent `[mqtt]`
section means MQTT is disabled).

---

## Topics

| Topic | QoS | Retain | Content |
|-------|-----|--------|---------|
| `sitta/{station_id}/status` | 1 | yes | `{"state":"online"}` / LWT `{"state":"offline"}` |
| `sitta/{station_id}/detection` | 0 | no | Full DetectionEvent JSON on every detection |
| `sitta/{station_id}/first_today/{scientific_name}` | 1 | yes | First confident detection per species per calendar day |
| `sitta/{station_id}/first_today` | 1 | yes | Latest first-of-day event (for HA discovery sensor) |
| `sitta/{station_id}/species_today` | 1 | yes | `{"count": N, "day": "2026-04-17"}` |

Scientific names in topic paths are lowercased with spaces → underscores.

## First-of-Day Tracking

In-memory `HashSet<String>` keyed by scientific name. Resets lazily on
the first detection of a new calendar day (in the station's configured
timezone via `chrono-tz`). No midnight timer — just compare
`today != current_date` on each event.

Separate confidence threshold (`first_of_day_min_confidence`, default
0.75) — higher than display threshold to reduce false "first of day"
announcements.

On restart, the set starts empty. The first detection of each species
will re-trigger as first-of-day. This is acceptable — the retained
MQTT message is simply updated and HA automations re-fire.

## Home Assistant Auto-Discovery

Published on every MQTT connect/reconnect:

1. **Binary sensor**: station online/offline status
2. **Sensor**: latest detection (common name + JSON attributes)
3. **Sensor**: first bird of day (common name from latest first-of-day)
4. **Sensor**: species count today

All sensors share a device block: `identifiers: ["sitta_{station_id}"]`.

## Config

```toml
[mqtt]
host = "localhost"
port = 1883
# username = "sitta"
# password = "secret"
first_of_day_min_confidence = 0.75
homeassistant_discovery = true
```

## Implementation

Module: `sitta-bin/src/mqtt.rs`
Dependencies: `rumqttc`, `chrono-tz` (sitta-bin only)

Two tokio tasks:
1. **Event loop**: drives `rumqttc::EventLoop::poll()`, handles ConnAck
   (publishes online status + HA discovery on connect/reconnect)
2. **Publisher**: subscribes to `broadcast::Sender<DetectionEvent>`,
   runs first-of-day tracker, publishes to MQTT topics

Follows the same background-task pattern as the snippet writer.

## Steps

1. Add MqttConfig to config.rs
2. Add rumqttc + chrono-tz dependencies
3. Implement mqtt.rs (FirstOfDayTracker, publisher, HA discovery)
4. Wire into main.rs (conditional spawn)
5. Tests for FirstOfDayTracker (pure logic, no I/O)
