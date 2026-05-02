use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use sitta_api::event::DetectionEvent;
use sitta_api::settings::RuntimeSettings;
use sitta_audio::source::SourceConfig;
use sitta_inference::model::Classifier;
use sitta_store::db::Database;
use sitta_store::matcher::IndividualMatcher;
use sitta_store::models::{NewAudioSource, NewLabel, NewModel, NewStation};
use sitta_taxonomy::EbirdTaxonomy;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config::Config;
use crate::persist::{PersistCtx, PresenceTracker};

/// Stable namespace for deriving deterministic UUIDs from config strings.
pub const SITTA_NS: Uuid = Uuid::from_bytes([
    0x91, 0x7a, 0x5c, 0x3e, 0x8b, 0x2d, 0x4f, 0x01,
    0xa6, 0x78, 0x3d, 0x9e, 0x5b, 0x7c, 0x1a, 0x42,
]);

/// Seed reference data (station, sources, models, labels) and build
/// the persistence context that consumers use for detection writes.
pub async fn seed_database(
    db: &Database,
    config: &Config,
    classifiers: &[Arc<dyn Classifier>],
    perch: Option<&Arc<dyn Classifier>>,
    taxonomy: Option<&EbirdTaxonomy>,
    settings: Arc<ArcSwap<RuntimeSettings>>,
) -> Result<PersistCtx> {
    let station_id = Uuid::new_v5(&SITTA_NS, config.station.id.as_bytes());
    db.upsert_station(&NewStation {
        id: &station_id,
        name: &config.station.name,
        latitude: config.station.latitude.map(f64::from),
        longitude: config.station.longitude.map(f64::from),
    })
    .await?;

    let mut source_ids: HashMap<String, Uuid> = HashMap::new();
    for source in &config.audio.sources {
        let name = source.name();
        let source_id = Uuid::new_v5(&station_id, name.as_bytes());
        let (source_type, uri, sample_rate, channels) = match source {
            SourceConfig::Rtsp(c) => ("rtsp", Some(c.url.as_str()), c.sample_rate, c.channels),
            SourceConfig::Local(c) => {
                ("local", Some(c.device.as_str()), c.sample_rate, c.channels)
            }
            SourceConfig::Remote(c) => {
                ("remote", Some(c.url.as_str()), 48000, 1)
            }
        };
        db.upsert_audio_source(&NewAudioSource {
            id: &source_id,
            station_id: &station_id,
            name,
            source_type,
            uri,
            sample_rate: i64::from(sample_rate),
            channels: i64::from(channels),
        })
        .await?;
        source_ids.insert(name.to_string(), source_id);
    }

    let mut model_ids: HashMap<String, i64> = HashMap::new();
    for classifier in classifiers.iter().chain(perch) {
        let model_id = seed_model(db, classifier.as_ref(), taxonomy).await?;
        model_ids.insert(classifier.name().to_string(), model_id);
    }

    let label_cache = db.load_label_id_cache().await?;
    tracing::info!(
        models = model_ids.len(),
        labels = label_cache.len(),
        sources = source_ids.len(),
        "Database seeded"
    );

    let (detection_tx, _) = broadcast::channel::<DetectionEvent>(64);

    // Individual matcher — threshold from Perch config, or default 0.85.
    let individual_threshold = config
        .inference
        .perch
        .as_ref()
        .map(|p| p.individual_threshold)
        .unwrap_or(0.85);
    let matcher = IndividualMatcher::new(db.clone(), individual_threshold).await?;

    let presence_tracker = PresenceTracker::new(
        config.presence.min_detections,
        config.presence.window_minutes,
    );

    Ok(PersistCtx {
        db: db.clone(),
        label_cache: Arc::new(label_cache),
        model_ids: Arc::new(model_ids),
        source_ids: Arc::new(source_ids),
        station_id,
        detection_tx,
        matcher: Some(Arc::new(matcher)),
        settings: settings.clone(),
        snippet_writer: None, // set by main after spawning the writer
        broadcast_dedup: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        range_filter: None,   // set by main after loading models
        station_latitude: settings.load().station_latitude,
        api_base_url: None,   // set by main from config
        presence_tracker: std::sync::Arc::new(std::sync::Mutex::new(presence_tracker)),
    })
}

async fn seed_model(
    db: &Database,
    classifier: &dyn Classifier,
    taxonomy: Option<&EbirdTaxonomy>,
) -> Result<i64> {
    let (model_name, model_version) = parse_model_name(classifier.name());
    let emb_dim = classifier.embedding_dim();
    let model_id = db
        .upsert_model(&NewModel {
            name: model_name,
            version: model_version,
            sample_rate: classifier.sample_rate() as i64,
            window_samples: classifier.window_samples() as i64,
            has_embeddings: emb_dim.is_some(),
            embedding_dim: emb_dim.map(|d| d as i64),
        })
        .await?;

    let raw_labels = classifier.raw_labels();
    if raw_labels.is_empty() {
        return Ok(model_id);
    }

    let label_entries: Vec<_> = raw_labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let (scientific_name, common_name, taxon_code) =
                parse_label_for_seeding(label, taxonomy);
            (i, scientific_name, common_name, taxon_code)
        })
        .collect();

    let new_labels: Vec<NewLabel<'_>> = label_entries
        .iter()
        .map(|(i, sci, common, taxon)| NewLabel {
            model_id,
            label_index: *i as i64,
            scientific_name: sci.as_deref(),
            common_name: common,
            label_type: if sci.is_some() {
                "species"
            } else {
                "environment"
            },
            taxon_code: taxon.as_deref(),
        })
        .collect();

    db.seed_labels(&new_labels).await?;
    tracing::info!(
        model = classifier.name(),
        labels = new_labels.len(),
        "Seeded model labels"
    );

    Ok(model_id)
}

/// Map classifier display name to (db_name, db_version).
pub fn parse_model_name(name: &str) -> (&str, &str) {
    match name {
        "BirdNET v2.4" => ("birdnet", "2.4"),
        "BirdNET v3.0" => ("birdnet", "3.0"),
        "Perch v2" => ("perch", "2"),
        "BSG Finland" => ("bsg_finland", "4.4"),
        _ => ("unknown", "0"),
    }
}

pub fn parse_label_for_seeding(
    label: &str,
    taxonomy: Option<&EbirdTaxonomy>,
) -> (Option<String>, String, Option<String>) {
    if let Some(entry) = taxonomy.and_then(|t| t.lookup(label)) {
        return (
            Some(entry.scientific_name.clone()),
            entry.common_name.clone(),
            Some(entry.species_code.clone()),
        );
    }
    if let Some((sci, common)) = label.split_once('_') {
        let taxon_code = taxonomy
            .and_then(|t| t.lookup(sci))
            .map(|e| e.species_code.clone());
        return (Some(sci.to_string()), common.to_string(), taxon_code);
    }
    // Fallback: a label like "Dryobates villosus" — a binomial scientific
    // name with no common-name suffix, and not in our taxonomy (e.g. a
    // recent taxonomic revision the eBird CSV doesn't yet carry). Treat
    // it as a scientific name so that links and bucketing key off the
    // right field; common_name mirrors it so the row is still rendered
    // (the cross-row enrichment migration will replace it with a sister
    // row's proper common name when one exists).
    if looks_like_binomial(label) {
        return (Some(label.to_string()), label.to_string(), None);
    }
    (None, label.to_string(), None)
}

/// Loose binomial detector: "Genus species" — Capitalized first word
/// followed by an all-lowercase second word (with optional hyphen). Used
/// to recognise scientific names that arrive without a common-name suffix.
fn looks_like_binomial(s: &str) -> bool {
    let mut parts = s.split_whitespace();
    let Some(genus) = parts.next() else { return false };
    let Some(species) = parts.next() else { return false };
    if parts.next().is_some() {
        return false;
    }
    let mut g = genus.chars();
    let Some(first) = g.next() else { return false };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if !g.all(|c| c.is_ascii_lowercase()) {
        return false;
    }
    !species.is_empty() && species.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use sitta_taxonomy::TaxonEntry;

    #[test]
    fn parse_model_name_known() {
        assert_eq!(parse_model_name("BirdNET v2.4"), ("birdnet", "2.4"));
        assert_eq!(parse_model_name("BirdNET v3.0"), ("birdnet", "3.0"));
        assert_eq!(parse_model_name("Perch v2"), ("perch", "2"));
        assert_eq!(parse_model_name("BSG Finland"), ("bsg_finland", "4.4"));
    }

    #[test]
    fn parse_model_name_unknown() {
        assert_eq!(parse_model_name("FutureModel v99"), ("unknown", "0"));
    }

    #[test]
    fn parse_label_birdnet_format_no_taxonomy() {
        let (sci, common, taxon) = parse_label_for_seeding("Tyto alba_Barn Owl", None);
        assert_eq!(sci, Some("Tyto alba".into()));
        assert_eq!(common, "Barn Owl");
        assert_eq!(taxon, None);
    }

    #[test]
    fn parse_label_perch_format_with_taxonomy() {
        let tax = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "barowl1".into(),
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
        }]);
        let (sci, common, taxon) = parse_label_for_seeding("Tyto_alba", Some(&tax));
        assert_eq!(sci, Some("Tyto alba".into()));
        assert_eq!(common, "Barn Owl");
        assert_eq!(taxon, Some("barowl1".into()));
    }

    #[test]
    fn parse_label_non_species() {
        let (sci, common, taxon) = parse_label_for_seeding("Engine", None);
        assert_eq!(sci, None);
        assert_eq!(common, "Engine");
        assert_eq!(taxon, None);
    }

    #[test]
    fn parse_label_binomial_no_underscore_no_taxonomy() {
        // Label like "Dryobates villosus" with no common-name suffix and
        // not in the (absent) taxonomy. Must still produce a usable
        // scientific_name so links and bucketing work.
        let (sci, common, taxon) = parse_label_for_seeding("Dryobates villosus", None);
        assert_eq!(sci, Some("Dryobates villosus".into()));
        assert_eq!(common, "Dryobates villosus");
        assert_eq!(taxon, None);
    }

    #[test]
    fn parse_label_binomial_skipped_for_common_names() {
        // "Hairy Woodpecker" is two Title-Case words — not a binomial.
        // Stays as an environment-type label.
        let (sci, common, _) = parse_label_for_seeding("Hairy Woodpecker", None);
        assert_eq!(sci, None);
        assert_eq!(common, "Hairy Woodpecker");
    }

    #[test]
    fn looks_like_binomial_cases() {
        assert!(looks_like_binomial("Dryobates villosus"));
        assert!(looks_like_binomial("Junco hyemalis-oreganus"));
        assert!(!looks_like_binomial("Hairy Woodpecker"));
        assert!(!looks_like_binomial("Engine"));
        assert!(!looks_like_binomial("Genus species subspecies"));
        assert!(!looks_like_binomial(""));
        assert!(!looks_like_binomial("genus species")); // genus must be capitalised
    }

    #[test]
    fn parse_label_birdnet_with_taxonomy_enrichment() {
        let tax = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "barowl1".into(),
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
        }]);
        let (sci, common, taxon) = parse_label_for_seeding("Tyto alba_Barn Owl", Some(&tax));
        assert_eq!(sci, Some("Tyto alba".into()));
        assert_eq!(common, "Barn Owl");
        assert_eq!(taxon, Some("barowl1".into()));
    }
}
