use std::sync::atomic::Ordering;
use std::sync::Arc;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use sitta_api::server::PipelineMetrics;
use sitta_audio::chunk::AudioChunk;
use sitta_inference::model::{Classifier, Classification};
use sitta_inference::rangefilter::RangeFilter;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::persist::{self, PersistCtx};

/// Spawn a background task that buffers 48 kHz chunks, resamples to 32 kHz,
/// and runs Perch inference on 5-second windows with 3-second stride.
pub fn spawn_perch_consumer(
    model: Arc<dyn Classifier>,
    range_filter: Option<Arc<RangeFilter>>,
    mut rx: broadcast::Receiver<Arc<AudioChunk>>,
    shutdown: CancellationToken,
    persist: PersistCtx,
    metrics: Arc<PipelineMetrics>,
) {
    const WINDOW_SAMPLES_IN: usize = 240_000;
    const STRIDE_SAMPLES: usize = 144_000;

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
                                metrics.perch_chunks_processed.fetch_add(1, Ordering::Relaxed);
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
                                            log_detections(&source_name, &chunk_id, &model_display, &detections);
                                            if let Some(mid) = model_id {
                                                persist::persist_detections(
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
                            metrics.perch_chunks_dropped.fetch_add(n, Ordering::Relaxed);
                            tracing::warn!(
                                dropped = n,
                                total_dropped = metrics.perch_chunks_dropped.load(Ordering::Relaxed),
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

/// Process a single audio chunk through all BirdNET classifiers.
pub async fn handle_chunk(
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
                    log_detections(&source_name, &chunk_id, &model_name, &detections);
                    if let Some(&model_id) = persist.model_ids.get(&model_name) {
                        persist::persist_detections(persist, model_id, &model_name, chunk, &detections, None).await;
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

fn log_detections(
    source_name: &str,
    chunk_id: &uuid::Uuid,
    model_name: &str,
    detections: &[Classification],
) {
    for d in detections {
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
}
