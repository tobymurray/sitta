# Phase 2: API & Live Dashboard

Expose detections over HTTP with a real-time live feed and a lightweight
embedded dashboard. No JavaScript build chain — htmx + SSE.

---

## Architecture

```
                            ┌─────────────────────┐
inference consumers ──────▶ │  persist_detections  │
                            │                      │
                            │  1. SQLite INSERT     │
                            │  2. broadcast::send   │──▶ detection_tx
                            └─────────────────────┘
                                                        │
                    ┌───────────────────────────────────┘
                    │
                    ▼
        ┌───────────────────────────────────────┐
        │          sitta-api (axum)              │
        │                                       │
        │  GET /api/v1/stream/events  ◀── SSE   │ ← browser subscribes
        │  GET /api/v1/detections     ◀── REST  │ ← paginated history
        │  GET /api/v1/species        ◀── REST  │ ← species summary
        │  GET /api/v1/detections/:id ◀── REST  │ ← detail + predictions
        │  GET /api/v1/status         ◀── REST  │ ← system health
        │  GET /                      ◀── HTML  │ ← embedded dashboard
        └───────────────────────────────────────┘
```

Two data paths:

1. **Live feed (SSE):** A `tokio::sync::broadcast` channel carries detection
   events from inference to the API. Each connected client gets an SSE stream
   of detection events as they happen. No polling, no WebSocket complexity.

2. **Historical queries (REST):** axum handlers query SQLite via the existing
   `Database` pool. Paginated, filtered by date range and species.

---

## Step 1 — Detection event broadcast

Add a broadcast channel for detection events alongside the existing audio
broadcast. This is the bridge between inference and the API.

### DetectionEvent type

Define in `sitta-api` (or a shared location) so both the inference pipeline
and the API can use it:

```rust
/// A detection event for live streaming and API responses.
/// Serialized to JSON for SSE and REST.
#[derive(Clone, Serialize)]
pub struct DetectionEvent {
    pub id: String,                     // UUIDv7 as hyphenated string
    pub detected_at: String,            // ISO 8601
    pub station_id: String,
    pub source_name: Option<String>,
    pub model: String,                  // "birdnet", "perch"
    pub model_version: String,          // "2.4", "2"
    pub species: SpeciesInfo,
    pub confidence: f32,
    pub alternatives: Vec<Alternative>, // top-k secondary predictions
    pub has_embedding: bool,
}

#[derive(Clone, Serialize)]
pub struct SpeciesInfo {
    pub scientific_name: String,
    pub common_name: String,
    pub taxon_code: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Alternative {
    pub rank: u32,
    pub scientific_name: String,
    pub common_name: String,
    pub confidence: f32,
}
```

### Wire into PersistCtx

Add a `broadcast::Sender<DetectionEvent>` to `PersistCtx`. In
`persist_detections`, after the successful SQLite insert, send the event:

```rust
let _ = ctx.detection_tx.send(event);  // Ok to drop if no receivers
```

Channel capacity: 64 is plenty. Slow consumers (lagging SSE clients) get
dropped events — they can catch up from the REST history endpoint.

---

## Step 2 — sitta-api crate: axum server

### Dependencies

```toml
[dependencies]
axum = { version = "0.8", features = ["macros"] }
tokio = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sitta-store = { path = "../sitta-store" }
uuid = { workspace = true }
chrono = { workspace = true }
tower-http = { version = "0.6", features = ["cors", "fs"] }
```

### Server setup

```rust
pub struct ApiServer {
    db: Database,
    detection_rx: broadcast::Sender<DetectionEvent>,  // clone to subscribe
    station_name: String,
}
```

`ApiServer` is built in `main.rs` after database setup and passed to the
axum router as state. The `broadcast::Sender` is cheaply clonable — each SSE
handler calls `.subscribe()` to get its own receiver.

### Router

```rust
let app = Router::new()
    // API endpoints
    .route("/api/v1/stream/events", get(sse_handler))
    .route("/api/v1/detections", get(list_detections))
    .route("/api/v1/detections/{id}", get(get_detection))
    .route("/api/v1/species", get(list_species))
    .route("/api/v1/status", get(get_status))
    // Dashboard (embedded HTML)
    .fallback(get(dashboard_handler))
    .with_state(state);
```

### Config

Add an `[api]` section to `config.toml`:

```toml
[api]
bind = "0.0.0.0:8080"
```

Default: `0.0.0.0:8080`. The server binds on startup alongside the inference
pipeline.

---

## Step 3 — SSE live feed

The core feature. A browser connects to `/api/v1/stream/events` and receives
detection events as they happen.

### Handler

```rust
async fn sse_handler(
    State(state): State<ApiState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.detection_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap();
                    yield Ok(Event::default().event("detection").data(json));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, "SSE client lagged");
                    // Continue — client catches up from REST if needed
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
    )
}
```

SSE keep-alive every 15s prevents proxies and browsers from closing idle
connections.

### Client-side (htmx)

```html
<div hx-ext="sse"
     sse-connect="/api/v1/stream/events"
     sse-swap="detection"
     hx-swap="afterbegin">
</div>
```

Each `detection` event's data is an HTML fragment rendered server-side and
swapped into the DOM. The server sends two representations:

- **JSON** for the `/api/v1/stream/events` endpoint (API consumers, MQTT)
- **HTML fragment** for the dashboard SSE endpoint (htmx)

We can either have two SSE endpoints (one JSON, one HTML) or a single
endpoint with content negotiation via `Accept` header. Two endpoints is
simpler:

- `/api/v1/stream/events` — JSON (API, scripts, MQTT bridge)
- `/stream` — HTML fragments (htmx dashboard)

---

## Step 4 — REST endpoints

### GET /api/v1/detections

Paginated recent detections. Query parameters:

- `since` — Unix ms timestamp (default: 24 hours ago)
- `until` — Unix ms timestamp (default: now)
- `species` — filter by scientific name
- `model` — filter by model name
- `source` — filter by source name
- `limit` — max results (default: 50, max: 500)
- `offset` — pagination offset

Returns JSON array of detection objects (same shape as the SSE event, plus
secondary predictions).

### GET /api/v1/detections/{id}

Single detection with full detail: primary + secondary predictions, model
info, source info. The `id` is a UUIDv7 string.

### GET /api/v1/species

Species summary for a date range. Returns distinct species with detection
counts, most recent detection timestamp, and average confidence.

Query parameters: `since`, `until`.

```json
[
  {
    "scientific_name": "Tyto alba",
    "common_name": "Barn Owl",
    "taxon_code": "barowl1",
    "detection_count": 42,
    "last_detected_at": "2026-04-17T14:30:00Z",
    "avg_confidence": 0.87
  }
]
```

### GET /api/v1/status

System health: station name, uptime, model info, source status, database
size, detection count. Lightweight — no heavy queries.

---

## Step 5 — Read queries in sitta-store

The existing `Database` has write methods. Add read methods for the API:

```rust
// Recent detections with label/model/source info joined
pub async fn recent_detections(&self, since: i64, until: i64, limit: i64, offset: i64)
    -> Result<Vec<DetectionRow>, StoreError>

// Single detection with full joins
pub async fn get_detection(&self, id: &[u8])
    -> Result<Option<DetectionDetailRow>, StoreError>

// Predictions for a detection
pub async fn get_predictions(&self, detection_id: &[u8])
    -> Result<Vec<PredictionRow>, StoreError>

// Species summary for date range
pub async fn species_summary(&self, since: i64, until: i64)
    -> Result<Vec<SpeciesSummaryRow>, StoreError>

// Detection count and DB stats
pub async fn stats(&self) -> Result<StatsRow, StoreError>
```

These use the indexes designed in the schema:
- `idx_detections_detected_at` for recent detections
- `idx_detections_label_time` for species history
- `idx_detections_time_label` for daily species list

All queries use `sqlx::query!` for compile-time checking.

---

## Step 6 — Embedded dashboard (htmx)

A single-page dashboard served from the binary. No npm, no bundler, no
framework. htmx + a CSS classless framework (Pico CSS or Simple.css).

### Pages / views

**Live feed (default view):**
- Auto-updating list of recent detections via SSE
- Each detection card shows: species name, confidence bar, model badge,
  timestamp, source name
- New detections slide in at the top
- Color-coded confidence (green >0.8, yellow >0.5, red <0.5)

**Species list:**
- Table of species detected today (or configurable date range)
- Columns: species, count, last seen, avg confidence
- Click a species to filter the live feed

**Status:**
- Station name and location
- Model versions loaded
- Audio source status (connected/reconnecting)
- Database size, total detections
- System uptime

### Serving strategy

Embed HTML templates in the binary via `include_str!`. For Pico CSS, either
embed the minified CSS (~10 KB) or serve from a CDN link. Embedding is
better for the local-first philosophy.

Templates are simple string formatting — no template engine needed for v1.
If complexity grows, `askama` (compile-time Jinja2 templates) is the natural
upgrade.

### HTML structure

```
GET /              → full page (shell + initial content)
GET /stream        → SSE endpoint returning HTML fragments
GET /species       → full page with species table
GET /status        → full page with system info
```

The shell page includes:
```html
<script src="https://unpkg.com/htmx.org@2/dist/htmx.min.js"></script>
<script src="https://unpkg.com/htmx-ext-sse@2/sse.js"></script>
```

Or embed htmx.min.js (~14 KB) in the binary for fully offline operation.

---

## Step 7 — Wire into main.rs

Bring it together in `sitta-bin/src/main.rs`:

```rust
// After database setup and model loading:
let (detection_tx, _) = broadcast::channel::<DetectionEvent>(64);

// Add detection_tx to PersistCtx
let persist_ctx = PersistCtx {
    db: db.clone(),
    detection_tx: detection_tx.clone(),
    // ... existing fields
};

// Start API server
let api_server = ApiServer::new(db.clone(), detection_tx.clone(), &config);
let api_shutdown = shutdown.clone();
tokio::spawn(async move {
    api_server.run(api_shutdown).await;
});

// Inference consumers continue as before — persist_detections now also
// broadcasts to the SSE channel.
```

Startup order:
1. Load config, taxonomy, models
2. Open database, seed reference data
3. Start API server (begins accepting connections)
4. Start audio capture + inference consumers
5. Detections flow: inference → SQLite + broadcast → SSE clients

---

## Step 8 — MQTT publish (optional, can defer)

Add an MQTT client that subscribes to the same detection broadcast channel
and publishes to `sitta/{station_id}/detection`. Home Assistant
auto-discovery messages are sent on connect.

This is independent of the HTTP API and can be implemented in parallel or
deferred to a later step. The broadcast channel architecture supports
multiple subscribers without changes.

---

## Implementation order

| Step | What                                    | Crate changes           |
|------|-----------------------------------------|-------------------------|
| 1    | DetectionEvent type + broadcast channel | sitta-api, sitta-bin    |
| 2    | axum server skeleton + config           | sitta-api, sitta-bin    |
| 3    | SSE live feed endpoint                  | sitta-api               |
| 4    | REST endpoints (detections, species)    | sitta-api               |
| 5    | Read queries in sitta-store             | sitta-store             |
| 6    | Embedded htmx dashboard                 | sitta-api               |
| 7    | Wire into main.rs                       | sitta-bin               |
| 8    | MQTT publish (defer ok)                 | sitta-api or sitta-bin  |

Steps 1-3 are the critical path to "I can see what's being detected now
in a browser." Steps 4-5 add historical depth. Step 6 makes it look
presentable. Step 7 is integration. Step 8 is independent.

---

## What this plan does NOT cover

- **Audio snippet serving** — Saving WAV files and serving them via the API
  for playback in the dashboard. Prerequisite: decide on directory layout
  and retention policy. Natural follow-on after the dashboard exists.
- **Spectrogram visualization** — Generating spectrograms for the UI
  (`rustfft` + image rendering). Phase 4 work.
- **Authentication** — Not needed for a local network device. If exposed
  publicly, add later behind a reverse proxy or with a simple token.
- **HTTPS** — Same: reverse proxy (caddy, nginx) handles TLS termination.
  The axum server binds plain HTTP on localhost/LAN.
- **Individual recognition UI** — Enrollment, matching display, individual
  timelines. Phase 3, requires the embedding comparison logic first.
