//! axum HTTP server with SSE live feed and REST endpoints.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::event::DetectionEvent;
use sitta_store::db::Database;

/// Shared state for all axum handlers.
#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    /// Clone of the broadcast sender — call `.subscribe()` per SSE client.
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    pub station_name: String,
}

/// Build the axum router with all routes.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/stream/events", get(sse_handler))
        .route("/api/v1/status", get(status_handler))
        .with_state(state)
}

/// Bind and serve until the shutdown token is cancelled.
pub async fn serve(addr: SocketAddr, state: ApiState, shutdown: CancellationToken) {
    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %addr, error = %e, "Failed to bind API server");
            return;
        }
    };
    tracing::info!(%addr, "API server listening");
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;
}

// ── SSE live feed ───────────────────────────────────────────────

async fn sse_handler(
    State(state): State<ApiState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.detection_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok::<_, Infallible>(
                            Event::default().event("detection").data(json)
                        );
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, "SSE client lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ── Status ──────────────────────────────────────────────────────

async fn status_handler(State(state): State<ApiState>) -> Json<StatusResponse> {
    Json(StatusResponse {
        station_name: state.station_name.clone(),
        status: "running",
    })
}

#[derive(Serialize)]
struct StatusResponse {
    station_name: String,
    status: &'static str,
}
