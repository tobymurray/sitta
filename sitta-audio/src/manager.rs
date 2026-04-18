//! Dynamic audio source lifecycle management.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;

use crate::chunk::AudioChunk;
use crate::remote::RemoteSource;
use crate::rtsp::RtspSource;
use crate::source::SourceConfig;

/// Manages audio source tasks with dynamic add/remove at runtime.
#[derive(Clone)]
pub struct SourceManager {
    inner: Arc<Inner>,
}

struct Inner {
    sources: RwLock<HashMap<String, ActiveSource>>,
    tx: broadcast::Sender<Arc<AudioChunk>>,
    shutdown: CancellationToken,
    chunk_seconds: u32,
}

struct ActiveSource {
    config: SourceConfig,
    cancel: CancellationToken,
}

impl SourceManager {
    pub fn new(
        tx: broadcast::Sender<Arc<AudioChunk>>,
        shutdown: CancellationToken,
        chunk_seconds: u32,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                sources: RwLock::new(HashMap::new()),
                tx,
                shutdown,
                chunk_seconds,
            }),
        }
    }

    /// Spawn a source from config. Returns error if name already active.
    pub async fn add(&self, config: SourceConfig) -> Result<(), String> {
        let name = config.name().to_string();
        let mut sources = self.inner.sources.write().await;

        if sources.contains_key(&name) {
            return Err(format!("source '{name}' already exists"));
        }

        let cancel = self.inner.shutdown.child_token();
        spawn_source(&config, self.inner.tx.clone(), self.inner.chunk_seconds, cancel.clone());

        sources.insert(name.clone(), ActiveSource { config, cancel });
        tracing::info!(source = %name, "Audio source added");
        Ok(())
    }

    /// Stop and remove a source by name.
    pub async fn remove(&self, name: &str) -> Result<SourceConfig, String> {
        let mut sources = self.inner.sources.write().await;
        let active = sources
            .remove(name)
            .ok_or_else(|| format!("source '{name}' not found"))?;
        active.cancel.cancel();
        tracing::info!(source = %name, "Audio source removed");
        Ok(active.config)
    }

    /// List all active source configs.
    pub async fn list(&self) -> Vec<SourceConfig> {
        let sources = self.inner.sources.read().await;
        sources.values().map(|a| a.config.clone()).collect()
    }

    /// List active source names.
    pub async fn names(&self) -> Vec<String> {
        let sources = self.inner.sources.read().await;
        sources.keys().cloned().collect()
    }

    /// Check if a source name exists.
    pub async fn contains(&self, name: &str) -> bool {
        let sources = self.inner.sources.read().await;
        sources.contains_key(name)
    }

    /// Spawn all sources from initial config.
    pub async fn add_initial(&self, configs: &[SourceConfig]) {
        for config in configs {
            if let Err(e) = self.add(config.clone()).await {
                tracing::error!(error = %e, "Failed to add initial source");
            }
        }
    }
}

fn spawn_source(
    config: &SourceConfig,
    tx: broadcast::Sender<Arc<AudioChunk>>,
    chunk_seconds: u32,
    cancel: CancellationToken,
) {
    match config {
        SourceConfig::Rtsp(rtsp_config) => {
            let source = RtspSource::new(rtsp_config.clone(), tx, chunk_seconds);
            tokio::spawn(async move {
                source.run(cancel).await;
            });
        }
        SourceConfig::Local(local_config) => {
            tracing::warn!(
                source = %local_config.name,
                "Local audio capture not yet implemented, skipping"
            );
        }
        SourceConfig::Remote(remote_config) => {
            let source = RemoteSource::new(remote_config.clone(), tx);
            tokio::spawn(async move {
                source.run(cancel).await;
            });
        }
    }
}
