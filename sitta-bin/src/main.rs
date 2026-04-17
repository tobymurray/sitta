mod config;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use sitta_audio::chunk::AudioChunk;
use sitta_audio::rtsp::RtspSource;
use sitta_audio::source::SourceConfig;
use sitta_inference::birdnet::BirdNet;
use sitta_inference::model::{Classification, Classifier};
use sitta_inference::rangefilter::RangeFilter;
use sitta_api::event::{Alternative, DetectionEvent, SpeciesInfo};
use sitta_store::db::Database;
use sitta_store::models::{
    NewAudioSource, NewDetection, NewLabel, NewModel, NewPrediction, NewStation,
};
use sitta_taxonomy::EbirdTaxonomy;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Config;

/// Stable namespace for deriving deterministic UUIDs from config strings.
const SITTA_NS: Uuid = Uuid::from_bytes([
    0x91, 0x7a, 0x5c, 0x3e, 0x8b, 0x2d, 0x4f, 0x01,
    0xa6, 0x78, 0x3d, 0x9e, 0x5b, 0x7c, 0x1a, 0x42,
]);

/// Shared context for persisting detections from any consumer.
#[derive(Clone)]
struct PersistCtx {
    db: Database,
    /// (model_db_id, label_index) → label_db_id
    label_cache: Arc<HashMap<(i64, i64), i64>>,
    /// classifier display name → model_db_id
    model_ids: Arc<HashMap<String, i64>>,
    /// source display name → source UUID
    source_ids: Arc<HashMap<String, Uuid>>,
    station_id: Uuid,
    /// Broadcast channel for live detection events (SSE, MQTT, etc.).
    detection_tx: broadcast::Sender<DetectionEvent>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("sitta=info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());
    let config_str = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read config file: {config_path}"))?;
    let config: Config =
        toml::from_str(&config_str).with_context(|| "failed to parse config file")?;

    tracing::info!(
        station_id = %config.station.id,
        station_name = %config.station.name,
        lat = config.station.latitude,
        lon = config.station.longitude,
        sources = config.audio.sources.len(),
        chunk_seconds = config.audio.chunk_seconds,
        "Starting Sitta"
    );

    // Load eBird taxonomy (optional).
    let taxonomy = load_taxonomy(&config)?;

    // Load BirdNET classifier and optional range filter together (needs concrete type for labels).
    let (birdnet_classifier, range_filter) = load_birdnet(&config, taxonomy.clone())?;

    let mut classifiers: Vec<Arc<dyn Classifier>> = Vec::new();
    if let Some(c) = birdnet_classifier {
        classifiers.push(c);
    }
    if classifiers.is_empty() {
        tracing::warn!("No inference models configured -- running in audio-only mode");
    }
    let classifiers: Arc<[Arc<dyn Classifier>]> = classifiers.into();
    let range_filter: Option<Arc<RangeFilter>> = range_filter.map(Arc::new);

    // Load Perch model (optional, no range filter — different label space).
    let perch_model = load_perch(&config, taxonomy.clone())?;

    // ── Database setup ──────────────────────────────────────────
    let db = Database::open(Path::new(&config.store.path))
        .await
        .context("failed to open database")?;
    tracing::info!(path = %config.store.path, "Database opened");

    let persist_ctx = seed_database(
        &db,
        &config,
        &classifiers,
        perch_model.as_ref(),
        taxonomy.as_deref(),
    )
    .await
    .context("failed to seed database")?;

    // ── Audio capture ───────────────────────────────────────────
    let (tx, _rx) = broadcast::channel::<Arc<AudioChunk>>(32);
    let shutdown = CancellationToken::new();

    for source_config in &config.audio.sources {
        match source_config {
            SourceConfig::Rtsp(rtsp_config) => {
                let source =
                    RtspSource::new(rtsp_config.clone(), tx.clone(), config.audio.chunk_seconds);
                let token = shutdown.clone();
                tokio::spawn(async move {
                    source.run(token).await;
                });
            }
            SourceConfig::Local(local_config) => {
                tracing::warn!(
                    source = %local_config.name,
                    "Local audio capture not yet implemented, skipping"
                );
            }
        }
    }

    // Spawn Perch consumer if configured.
    if let Some(perch) = perch_model {
        spawn_perch_consumer(
            perch,
            range_filter.clone(),
            tx.subscribe(),
            shutdown.clone(),
            persist_ctx.clone(),
        );
    }

    // Spawn BirdNET inference consumer.
    let mut rx = tx.subscribe();
    let consumer_shutdown = shutdown.clone();
    let consumer_classifiers = classifiers.clone();
    let consumer_filter = range_filter.clone();
    let consumer_persist = persist_ctx.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(chunk) => {
                            handle_chunk(&chunk, &consumer_classifiers, consumer_filter.clone(), &consumer_persist).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(dropped = n, "Inference consumer lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                () = consumer_shutdown.cancelled() => break,
            }
        }
    });

    // Wait for shutdown signal.
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for ctrl-c")?;
    tracing::info!("Shutting down...");
    shutdown.cancel();

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    db.close().await;

    Ok(())
}

// ── Database seeding ────────────────────────────────────────────

async fn seed_database(
    db: &Database,
    config: &Config,
    classifiers: &[Arc<dyn Classifier>],
    perch: Option<&Arc<dyn Classifier>>,
    taxonomy: Option<&EbirdTaxonomy>,
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
    for classifier in classifiers.iter().chain(perch.into_iter()) {
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

    Ok(PersistCtx {
        db: db.clone(),
        label_cache: Arc::new(label_cache),
        model_ids: Arc::new(model_ids),
        source_ids: Arc::new(source_ids),
        station_id,
        detection_tx,
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

fn parse_model_name(name: &str) -> (&str, &str) {
    match name {
        "BirdNET v2.4" => ("birdnet", "2.4"),
        "BirdNET v3.0" => ("birdnet", "3.0"),
        "Perch v2" => ("perch", "2"),
        "BSG Finland" => ("bsg_finland", "4.4"),
        _ => ("unknown", "0"),
    }
}

fn parse_label_for_seeding(
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
    (None, label.to_string(), None)
}

// ── Model loading ───────────────────────────────────────────────

fn load_taxonomy(config: &Config) -> Result<Option<Arc<EbirdTaxonomy>>> {
    let Some(tax_config) = &config.taxonomy else {
        return Ok(None);
    };
    let taxonomy = EbirdTaxonomy::load(Path::new(&tax_config.ebird_path))
        .with_context(|| format!("failed to load eBird taxonomy: {}", tax_config.ebird_path))?;
    Ok(Some(Arc::new(taxonomy)))
}

fn load_birdnet(
    config: &Config,
    taxonomy: Option<Arc<EbirdTaxonomy>>,
) -> Result<(Option<Arc<dyn Classifier>>, Option<RangeFilter>)> {
    let Some(birdnet_config) = &config.inference.birdnet else {
        return Ok((None, None));
    };

    let model = BirdNet::load_with_taxonomy(
        Path::new(&birdnet_config.model_path),
        Path::new(&birdnet_config.labels_path),
        birdnet_config.min_confidence,
        birdnet_config.top_k,
        taxonomy,
    )
    .context("failed to load BirdNET model")?;

    let range_filter = match (
        &birdnet_config.meta_model_path,
        config.station.latitude,
        config.station.longitude,
    ) {
        (Some(meta_path), Some(lat), Some(lon)) => {
            let force_allow = birdnet_config.force_allow.iter().cloned().collect();
            if !birdnet_config.force_allow.is_empty() && config.taxonomy.is_none() {
                tracing::warn!(
                    codes = ?birdnet_config.force_allow,
                    "force_allow requires [taxonomy] to resolve species codes — \
                     force_allow entries will have no effect without it"
                );
            }
            let filter = RangeFilter::load(
                Path::new(meta_path),
                model.labels(),
                lat,
                lon,
                birdnet_config.meta_threshold,
                force_allow,
            )
            .context("failed to load BirdNET range filter")?;
            Some(filter)
        }
        (Some(_), _, _) => {
            tracing::warn!(
                "meta_model_path is set but [station] latitude/longitude are missing — \
                 range filter disabled"
            );
            None
        }
        _ => None,
    };

    Ok((Some(Arc::new(model)), range_filter))
}

fn load_perch(
    config: &Config,
    taxonomy: Option<Arc<EbirdTaxonomy>>,
) -> Result<Option<Arc<dyn Classifier>>> {
    let Some(perch_config) = &config.inference.perch else {
        return Ok(None);
    };
    let model = BirdNet::load_with_taxonomy(
        Path::new(&perch_config.model_path),
        Path::new(&perch_config.labels_path),
        perch_config.min_confidence,
        perch_config.top_k,
        taxonomy,
    )
    .context("failed to load Perch model")?;
    Ok(Some(Arc::new(model)))
}

// ── Detection persistence ───────────────────────────────────────

async fn persist_detections(
    ctx: &PersistCtx,
    model_id: i64,
    classifier_name: &str,
    chunk: &AudioChunk,
    detections: &[Classification],
    embeddings: Option<&Vec<f32>>,
) {
    let top = match detections.first() {
        Some(d) => d,
        None => return,
    };
    let Some(&label_id) = ctx.label_cache.get(&(model_id, top.label_index as i64)) else {
        tracing::warn!(model_id, label_index = top.label_index, "Label not in cache");
        return;
    };

    let detection_id = Uuid::now_v7();
    let detected_at = chunk.captured_at.timestamp_millis();
    let source_id = ctx.source_ids.get(&chunk.source_name);

    if let Err(e) = ctx
        .db
        .insert_detection(&NewDetection {
            id: &detection_id,
            station_id: &ctx.station_id,
            source_id,
            model_id,
            label_id,
            detected_at,
            confidence: f64::from(top.confidence),
            snippet_path: None,
            snippet_duration_ms: None,
            snippet_sample_rate: None,
            metadata: None,
        })
        .await
    {
        tracing::error!(error = %e, "Failed to persist detection");
        return;
    }

    // Secondary predictions (rank 1+).
    let predictions: Vec<NewPrediction> = detections
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(r, p)| {
            let label_id = *ctx.label_cache.get(&(model_id, p.label_index as i64))?;
            Some(NewPrediction {
                rank: r as i64,
                label_id,
                confidence: f64::from(p.confidence),
            })
        })
        .collect();
    if let Err(e) = ctx.db.insert_predictions(&detection_id, &predictions).await {
        tracing::error!(error = %e, "Failed to persist predictions");
    }

    // Embedding (Perch path).
    let has_embedding = embeddings.is_some();
    if let Some(emb) = embeddings {
        if let Err(e) = ctx.db.insert_embedding(&detection_id, emb).await {
            tracing::error!(error = %e, "Failed to persist embedding");
        }
    }

    // Broadcast to live subscribers (SSE, MQTT).
    let (model_name, model_version) = parse_model_name(classifier_name);
    let alternatives: Vec<Alternative> = detections
        .iter()
        .enumerate()
        .skip(1)
        .map(|(r, c)| Alternative {
            rank: r as u32,
            scientific_name: c.species.scientific_name.clone(),
            common_name: c.species.common_name.clone(),
            confidence: c.confidence,
        })
        .collect();

    let event = DetectionEvent {
        id: detection_id.to_string(),
        detected_at: chunk.captured_at.to_rfc3339(),
        station_id: ctx.station_id.to_string(),
        source_name: Some(chunk.source_name.clone()),
        model: model_name.to_string(),
        model_version: model_version.to_string(),
        species: SpeciesInfo {
            scientific_name: top.species.scientific_name.clone(),
            common_name: top.species.common_name.clone(),
            taxon_code: top.species.taxon_code.clone(),
        },
        confidence: top.confidence,
        alternatives,
        has_embedding,
    };

    // Ok to drop if no receivers are subscribed yet.
    let _ = ctx.detection_tx.send(event);
}

// ── Inference consumers ─────────────────────────────────────────

fn spawn_perch_consumer(
    model: Arc<dyn Classifier>,
    range_filter: Option<Arc<RangeFilter>>,
    mut rx: broadcast::Receiver<Arc<AudioChunk>>,
    shutdown: CancellationToken,
    persist: PersistCtx,
) {
    const WINDOW_SAMPLES_IN: usize = 240_000;
    const STRIDE_SAMPLES: usize = 144_000;

    // Extract model metadata before moving `model` into the async block.
    let model_display = model.name().to_string();
    let model_id = persist.model_ids.get(model.name()).copied();

    tokio::spawn(async move {
        let mut resampler = Fft::<f32>::new(48_000, 32_000, 1024, 2, 1, FixedSync::Both)
            .expect("failed to create Perch resampler");

        let mut buf: Vec<f32> = Vec::with_capacity(WINDOW_SAMPLES_IN * 2);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(chunk) => {
                            buf.extend_from_slice(&chunk.samples);

                            while buf.len() >= WINDOW_SAMPLES_IN {
                                let window: Vec<f32> = buf[..WINDOW_SAMPLES_IN].to_vec();

                                resampler.reset();
                                let input_frames = WINDOW_SAMPLES_IN;
                                let input_adapter = InterleavedSlice::new(&window, 1, input_frames)
                                    .expect("invalid input adapter");
                                let output_capacity = resampler.process_all_needed_output_len(input_frames);
                                let mut output_buf = vec![0.0f32; output_capacity];
                                let mut output_adapter = InterleavedSlice::new_mut(
                                    &mut output_buf, 1, output_capacity,
                                )
                                .expect("invalid output adapter");
                                let (_in_used, out_produced) = resampler
                                    .process_all_into_buffer(
                                        &input_adapter,
                                        &mut output_adapter,
                                        input_frames,
                                        None,
                                    )
                                    .expect("resampling failed");
                                output_buf.truncate(out_produced);

                                let audio = output_buf;
                                let model_arc = model.clone();
                                let filter = range_filter.clone();
                                let source_name = chunk.source_name.clone();
                                let chunk_id = chunk.id;

                                let result = tokio::task::spawn_blocking(move || {
                                    let (detections, embeddings) =
                                        model_arc.classify_with_embeddings(&audio)?;
                                    let detections = if let Some(f) = filter.as_deref() {
                                        f.filter(detections)?
                                    } else {
                                        detections
                                    };
                                    Ok::<_, sitta_inference::InferenceError>((detections, embeddings))
                                })
                                .await;

                                match result {
                                    Ok(Ok((detections, embeddings))) => {
                                        if detections.is_empty() {
                                            tracing::debug!(
                                                source = %source_name,
                                                model = %model_display,
                                                "No Perch detections above threshold"
                                            );
                                        } else {
                                            for d in &detections {
                                                tracing::info!(
                                                    source = %source_name,
                                                    chunk_id = %chunk_id,
                                                    model = %model_display,
                                                    species = %d.species.common_name,
                                                    scientific_name = %d.species.scientific_name,
                                                    taxon_code = d.species.taxon_code.as_deref().unwrap_or(""),
                                                    confidence = format_args!("{:.3}", d.confidence),
                                                    "Detection"
                                                );
                                            }
                                            if let Some(mid) = model_id {
                                                persist_detections(
                                                    &persist,
                                                    mid,
                                                    &model_display,
                                                    &chunk,
                                                    &detections,
                                                    embeddings.as_ref(),
                                                )
                                                .await;
                                            }
                                        }
                                        if let Some(emb) = &embeddings {
                                            tracing::debug!(
                                                source = %source_name,
                                                chunk_id = %chunk_id,
                                                embedding_dim = emb.len(),
                                                "Perch embeddings available"
                                            );
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        tracing::error!(
                                            source = %source_name,
                                            error = %e,
                                            "Perch inference failed"
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            error = %e,
                                            "Perch inference task panicked"
                                        );
                                    }
                                }

                                buf.drain(..STRIDE_SAMPLES);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(
                                dropped = n,
                                "Perch consumer lagged, clearing buffer"
                            );
                            buf.clear();
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                () = shutdown.cancelled() => break,
            }
        }
    });
}

async fn handle_chunk(
    chunk: &AudioChunk,
    classifiers: &[Arc<dyn Classifier>],
    range_filter: Option<Arc<RangeFilter>>,
    persist: &PersistCtx,
) {
    if classifiers.is_empty() {
        tracing::info!(
            source = %chunk.source_name,
            chunk_id = %chunk.id,
            duration_s = format_args!("{:.1}", chunk.duration_secs()),
            rms_dbfs = format_args!("{:.1}", chunk.rms_dbfs()),
            "Audio chunk (no inference)"
        );
        return;
    }

    for classifier in classifiers {
        if chunk.samples.len() != classifier.window_samples() {
            tracing::debug!(
                source = %chunk.source_name,
                model = classifier.name(),
                expected = classifier.window_samples(),
                got = chunk.samples.len(),
                "Chunk size mismatch, skipping"
            );
            continue;
        }

        let samples = chunk.samples.clone();
        let model = classifier.clone();
        let filter = range_filter.clone();
        let source_name = chunk.source_name.clone();
        let chunk_id = chunk.id;
        let model_name = classifier.name().to_string();

        let result = tokio::task::spawn_blocking(move || {
            let detections = model.classify(&samples)?;
            if let Some(f) = filter.as_deref() {
                f.filter(detections)
            } else {
                Ok(detections)
            }
        })
        .await;

        match result {
            Ok(Ok(detections)) => {
                if detections.is_empty() {
                    tracing::debug!(
                        source = %source_name,
                        model = %model_name,
                        "No detections above threshold"
                    );
                } else {
                    for d in &detections {
                        tracing::info!(
                            source = %source_name,
                            chunk_id = %chunk_id,
                            model = %model_name,
                            species = %d.species.common_name,
                            scientific_name = %d.species.scientific_name,
                            taxon_code = d.species.taxon_code.as_deref().unwrap_or(""),
                            confidence = format_args!("{:.3}", d.confidence),
                            "Detection"
                        );
                    }
                    if let Some(&model_id) = persist.model_ids.get(&model_name) {
                        persist_detections(persist, model_id, &model_name, chunk, &detections, None).await;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!(
                    source = %source_name,
                    model = %model_name,
                    error = %e,
                    "Inference failed"
                );
            }
            Err(e) => {
                tracing::error!(
                    model = %model_name,
                    error = %e,
                    "Inference task panicked"
                );
            }
        }
    }
}
