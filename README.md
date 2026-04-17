# Sitta

A local-first, privacy-oriented bioacoustics engine written in Rust. Sitta replaces cloud-dependent bird identification services with an API-first design that runs entirely on your local network.

Named for the nuthatch genus (*Sitta*).

## Vision & Constraints

- **Local-only.** Every byte of audio, every embedding vector, every detection event stays on your network. No cloud component, no telemetry, no optional "enhanced" mode that uploads data.
- **Edge hardware.** Targets ARM64 SBCs (RPi 5, Orange Pi 5, Radxa Rock 5B) with 2-4 GB RAM.
- **No GPU assumed.** Optional Coral TPU via Edge TPU delegate.
- **Real-time.** Sustains inference on at least 2 concurrent audio streams.
- **Integration.** MQTT for Home Assistant, REST/WebSocket for dashboards.

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                        sitta (binary)                      │
│                                                            │
│   ┌───────────┐   ┌──────────────┐   ┌──────────────────┐  │
│   │  Audio    │   │  Inference   │   │   API / MQTT     │  │
│   │  Pipeline │─▶│  Engine      │─▶│   Gateway        │  │
│   │           │   │              │   │                  │  │
│   │ capture   │   │ birdnet      │   │ REST (axum)      │  │
│   │ resample  │   │ perch        │   │ WebSocket        │  │
│   │ buffer    │   │ individual   │   │ MQTT publish     │  │
│   │ dispatch  │   │ id matching  │   │ HA discovery     │  │
│   └───────────┘   └──────────────┘   └──────────────────┘  │
│         │                │                    │            │
│         ▼                ▼                    ▼            │
│   ┌───────────────────────────────────────────────────┐    │
│   │            sitta-store (SQLite + embeddings)      │    │
│   └───────────────────────────────────────────────────┘    │
│                              │                             │
│                              ▼                             │
│   ┌───────────────────────────────────────────────────┐    │
│   │          sitta-spatial (future: TDOA engine)      │    │
│   └───────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────┘
```

Data flows left-to-right through Tokio channels (`broadcast` for fan-out, `mpsc` for backpressure-aware point-to-point). No thread-per-stream -- the audio pipeline yields chunks into an async stream that the inference engine consumes.

## Workspace Structure

```
sitta/
├── Cargo.toml              # workspace root
├── config.toml             # runtime configuration
├── sitta-audio/            # audio capture, resampling, buffering
│   └── src/
│       ├── lib.rs
│       ├── chunk.rs        # AudioChunk type
│       ├── source.rs       # source config types (RTSP, local)
│       └── rtsp.rs         # ffmpeg-based RTSP capture
├── sitta-inference/        # model loading, inference, embedding ops
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # Classifier trait, Classification/Species types
│       └── birdnet.rs      # BirdNET inference via birdnet-onnx
├── sitta-store/            # persistence layer (stub)
├── sitta-spatial/          # future TDOA triangulation (stub)
├── sitta-api/              # HTTP, WebSocket, MQTT (stub)
└── sitta-bin/              # binary entry point, config, orchestration
    └── src/
        ├── main.rs
        └── config.rs
```

Workspace crates compile independently. When iterating on the API layer, `sitta-audio` and `sitta-inference` don't recompile. On ARM64 cross-compilation, this matters.

## Audio Capture

### RTSP via ffmpeg (primary path)

RTSP is the default input. ffmpeg runs as a subprocess, handling all codec negotiation and decoding. Sitta receives raw f32le PCM via a pipe. This means the RTSP stream can use any codec ffmpeg supports -- AAC, Opus, PCM, G.711, etc.

```
RTSP stream (any codec)          RTSP stream (any codec)
        │                                │
        ▼                                ▼
  ffmpeg (subprocess)              ffmpeg (subprocess)
  decode → f32le PCM               decode → f32le PCM
        │                                │
        ▼                                ▼
  ┌─ broadcast::channel (fan-out to all consumers) ─┐
  │                                                  │
  ▼                                                  ▼
BirdNET consumer (3s windows)        Perch consumer (5s windows, future)
```

- **One ffmpeg process per source.** Each RTSP stream gets its own ffmpeg subprocess with `kill_on_drop` for cleanup. If a process dies, the source reconnects with configurable backoff.
- **Codec-agnostic.** ffmpeg decodes to raw PCM before Sitta sees it. No codec libraries in the Rust binary.
- **Multiple sources.** Each source is a TOML array entry. Sources run as independent Tokio tasks. Scale is limited by CPU (each stream costs ~one core for inference at full rate).
- **Local audio also supported.** A `type = "local"` source variant exists for direct sound card capture (future implementation).

### Multi-Rate Pipeline (future)

BirdNET expects 48 kHz mono, 3-second windows. Google Perch expects 32 kHz mono, 5-second windows. ffmpeg can resample at capture time, or `rubato` with `SincFixedIn` can resample in-process for tighter control. Sinc interpolation is the right quality/cost tradeoff for bioacoustics where harmonics matter.

Every `AudioChunk` carries a `timestamp_ns: u64` (monotonic, relative to capture start), which is free now and required for TDOA later.

## Inference Engine

### Classifier Abstraction

The inference layer defines a `Classifier` trait that any model backend can implement:

```rust
pub trait Classifier: Send + Sync {
    fn classify(&self, audio: &[f32]) -> Result<Vec<Classification>>;
    fn name(&self) -> &str;
    fn sample_rate(&self) -> u32;
    fn window_samples(&self) -> usize;
}
```

Both BirdNET and Google Perch can serve as species classifiers. The architecture does not hardcode "BirdNET = species, Perch = embeddings." Instead:

| Capability | BirdNET | Perch |
|---|---|---|
| Species classification | Yes (built-in) | Yes (with classification head) |
| Embeddings for individual ID | No | Yes |
| Custom taxonomy / fine-tuning | Limited | Straightforward (train a head on embeddings) |

Multiple classifiers can run simultaneously. Detection events carry model provenance so you know which produced each result. Cross-referencing (BirdNET says "Barn Owl" at 0.85, Perch agrees at 0.90) gives a stronger signal than either alone.

### BirdNET-family models

Implemented via `birdnet-onnx` (wraps ONNX Runtime). Model type is auto-detected from tensor shape — supports BirdNET v2.4, v3.0, Perch v2, and BSG Finland. Sigmoid is applied internally.

**Getting a model:**

The quickest way is [`birda`](https://github.com/tphakala/birda), a CLI model manager by the birdnet-onnx author:
```bash
birda models install birdnet-v24
```
This downloads `birdnet.onnx` (converted by Justin Chu, hosted on HuggingFace at `justinchuby/BirdNET-onnx`) and the matching labels file.

Manual download options:

| Model | ONNX source | Labels |
|---|---|---|
| BirdNET v2.4 | [HuggingFace: justinchuby/BirdNET-onnx](https://huggingface.co/justinchuby/BirdNET-onnx) — use `birdnet.onnx` | [BirdNET-Analyzer labels](https://github.com/birdnet-team/BirdNET-Analyzer/tree/main/birdnet_analyzer/labels/V2.4) |
| BSG Finland v4.4 | [HuggingFace: tphakala/BSG](https://huggingface.co/tphakala/BSG) — `BSG_birds_Finland_v4_4_fused_fp32.onnx` | included in the HuggingFace repo |

Note: Zenodo only distributes BirdNET v2.4 in TFLite/Keras/Protobuf formats — no native ONNX. The `justinchuby/BirdNET-onnx` conversion via NVIDIA Nsight DL Designer is what makes it work (tf2onnx alone fails on the RFFT spectrogram ops, as documented in JOURNAL.md).

Once you have a model and labels file, configure in `config.toml`:
```toml
[inference.birdnet]
model_path = "/opt/sitta/models/birdnet.onnx"
labels_path = "/opt/sitta/models/BirdNET_GLOBAL_6K_V2.4_Labels_en_uk.txt"
min_confidence = 0.25
top_k = 10
```

### Google Perch v2

Implemented as a second `Classifier` backed by the same `birdnet-onnx` crate. Runs as an independent consumer alongside BirdNET.

- **Input:** 32 kHz mono, 5-second windows (160,000 samples)
- **Output:** species classifications (softmax) + 1536-dimensional embedding vector per window
- **Resampling:** incoming 48 kHz audio is resampled in-process via `rubato` (`Fft` resampler, 48000→32000 Hz, 5s windows, 3s stride / 2s overlap)
- **Install:** `birda models install perch-v2`

The 1536-dim embeddings are logged at `DEBUG` level. They will be stored in `sitta-store` in Phase 3 to enable individual animal identification via cosine similarity.

### birdnet-onnx backend

`birdnet-onnx` (by tphakala, the BirdNET-Go author) wraps ONNX Runtime with a purpose-built API for BirdNET-family models:

| Feature | birdnet-onnx |
|---|---|
| Model support | BirdNET v2.4, v3.0, Perch v2, BSG Finland |
| ONNX Runtime | Bundled at build time (or runtime `dlopen` via `load-dynamic` feature) |
| Thread safety | Internal `Arc` — no external Mutex needed |
| Hardware acceleration | CPU (default), CUDA, TensorRT, CoreML, ArmNN, XNNPACK, and more |
| Embeddings | Returned for v3.0 and Perch models |
| Labels | Parsed internally from the labels file |
| Sigmoid | Applied internally; no manual sensitivity tuning |

The `load-dynamic` feature mirrors how BirdNET-Go loads TFLite — useful for cross-compilation where you want to supply the ONNX Runtime `.so` at runtime rather than bundling it. The `cuda` feature enables GPU inference.

### Individual Identification (planned)

Perch embeddings enable recognising specific animals, not just species:

1. **Enrolment.** User labels a detection as "Barn Owl #1." The embedding vector is stored in `sitta-store`.
2. **Matching.** New embeddings are compared via cosine similarity against known individuals. Threshold: configurable, default 0.85.
3. **Brute-force search.** Dozens to low hundreds of individuals don't need a vector database. If the set grows, add HNSW (`instant-distance` crate).

Memory: 1280 x 4 bytes = 5 KB per embedding. 100 individuals x 5 references each = 2.5 MB. Negligible.

## Spatial Awareness (TDOA) -- Future

Phase 5 work. The architecture must not block it.

- 4 time-synchronised microphones, timestamps from the audio driver clock
- GCC-PHAT cross-correlation for time delays between mic pairs (4 mics = 6 pairs)
- Multilateration to resolve (x, y) position from TDOA + known mic geometry

## API Design

### REST Endpoints

```
GET  /api/v1/detections              # paginated detection history
GET  /api/v1/detections/:id          # single detection + audio snippet
GET  /api/v1/species                 # species list with detection counts
GET  /api/v1/individuals             # known individuals
POST /api/v1/individuals             # enrol new individual from detection
GET  /api/v1/stream/events           # WebSocket: live detection events
GET  /api/v1/status                  # system health, mic status, model info
POST /api/v1/config                  # runtime config update
```

### MQTT (Home Assistant)

Sitta publishes HA MQTT discovery messages on startup so detection sensors appear automatically. Detection events are published to `sitta/<station_id>/detection`.

## Detection Event Schema

```json
{
  "id": "01961074-...",
  "timestamp": "2026-04-15T08:30:00Z",
  "station_id": "station_01",
  "species": {
    "scientific_name": "Tyto alba",
    "common_name": "Barn Owl",
    "taxon_id": "barowl1",
    "model": "birdnet"
  },
  "confidence": 0.92,
  "individual": {
    "id": "a1b2c3d4-...",
    "label": "Barn Owl #1",
    "similarity": 0.91
  },
  "audio": {
    "sample_rate": 48000,
    "duration_ms": 3000,
    "channel_id": 0,
    "snippet_path": "/var/lib/sitta/snippets/01961074-....wav"
  },
  "location": null,
  "metadata": {
    "noise_floor_db": -42.5,
    "peak_frequency_hz": 3200.0,
    "inference_time_ms": 285
  }
}
```

Fields:

- **`species`** -- classification result with model provenance
- **`individual`** -- non-null when matched to a known individual via Perch embeddings
- **`location`** -- non-null when TDOA triangulation is available (Phase 5)
- **`metadata`** -- extensible key-value pairs for diagnostics

## ARM64 Hardware Considerations

### CPU Contention

BirdNET inference on a 3s window takes ~200-400ms on an RPi 5 (Cortex-A76). Perch is lighter (~100-200ms). Both models fit within a single core's budget but compete for L2 cache.

**Mitigation:** Pin each model to a dedicated core via `core_affinity`. Cores 2-3 for inference, core 0 for OS, core 1 for audio capture + API.

### Inference Backpressure

If the inference queue exceeds 2 pending chunks, drop the oldest. A stale window from 10 seconds ago is worth less than the current one. Log the drop as a metric.

### Coral TPU

A USB Coral runs BirdNET in ~15ms (vs. 300ms on CPU). Run BirdNET on Coral, Perch on CPU. Do not multiplex both models on one Coral -- context-switching overhead on the TPU kills throughput.

### Thermal Throttling

Sustained inference will trigger throttling without active cooling. Monitor `/sys/class/thermal/thermal_zone0/temp` and expose it on `/api/v1/status`. If temp exceeds 80C, reduce inference frequency.

## Configuration

```toml
[station]
id = "station_01"
name = "North Paddock"

[audio]
chunk_seconds = 3     # audio chunk duration (matches BirdNET window)

[[audio.sources]]
type = "rtsp"
name = "north_paddock"
url = "rtsp://192.168.1.100:554/stream1"
transport = "tcp"     # tcp (default) or udp
# sample_rate = 48000 # ffmpeg resamples if source differs
# channels = 1

[[audio.sources]]
type = "rtsp"
name = "south_dam"
url = "rtsp://192.168.1.101:554/stream1"

# [[audio.sources]]
# type = "local"
# name = "usb_mic"
# device = "M-Track Duo"

[inference.birdnet]
model_path = "/opt/sitta/models/birdnet_v2.4.onnx"
labels_path = "/opt/sitta/models/BirdNET_GLOBAL_6K_V2.4_Labels_en_uk.txt"
min_confidence = 0.25
top_k = 10

# Future sections (not yet implemented):
# [inference.perch]
# [api]
# [mqtt]
# [storage]
```

Runtime dependencies: `ffmpeg` must be installed on the host for RTSP capture.

## Dependencies

### Current (Phase 1)

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime |
| `tokio-util` | `CancellationToken` for graceful shutdown |
| `tracing` / `tracing-subscriber` | Async-aware structured logging |
| `serde` / `toml` | Configuration deserialisation |
| `chrono` | UTC timestamps |
| `uuid` | v7 time-sortable chunk/detection IDs |
| `thiserror` / `anyhow` | Error handling (library / binary) |
| `birdnet-onnx` | BirdNET/Perch species classification via ONNX Runtime |
| `rubato` | High-quality FFT-based audio resampling (48→32 kHz for Perch) |

### Planned (future phases)

| Crate | Purpose |
|---|---|
| `rubato` | High-quality sinc resampling (48->32 kHz) |
| `axum` | Tokio-native HTTP/WS server |
| `rumqttc` | Async MQTT client |
| `rusqlite` | SQLite with WAL mode, no ORM overhead |
| `cpal` | Local sound card capture (ALSA) |
| `core_affinity` | CPU pinning for deterministic scheduling |

## Design Decisions

| Anti-pattern avoided | What we do instead |
|---|---|
| gRPC / protobuf | JSON over REST + MQTT. No code generation step. |
| Heavy web framework | axum -- lightest Tokio-native option |
| ORM (Diesel, SeaORM) | Raw SQL via rusqlite. Simple schema, no abstraction tax. |
| Vector database | In-process brute-force cosine search. Hundreds of vectors don't need a server. |
| Docker on the SBC | Native binary. One binary, one config file, one systemd unit. |
| Plugin architecture | Workspace crates with clear boundaries. Plugins are YAGNI. |

## Roadmap

### Phase 1 -- Minimum Viable Listener

Capture audio, run BirdNET, emit detections.

- [x] Workspace skeleton (`sitta-audio`, `sitta-inference`, `sitta-store`, `sitta-api`, `sitta-spatial`, `sitta-bin`)
- [x] RTSP capture via ffmpeg subprocess with auto-reconnect
- [x] Multi-source configuration (`[[audio.sources]]`)
- [x] `config.toml` parsing
- [x] Structured logging (`tracing`)
- [x] Broadcast channel fan-out to multiple consumers
- [x] Classifier trait abstraction (supports BirdNET, future Perch)
- [x] BirdNET v2.4 inference via birdnet-onnx (ONNX Runtime)
- [x] Configurable confidence threshold and top_k
- [x] Inference runs on blocking threads (no async executor starvation)
- [x] Google Perch v2 inference (32 kHz, 5s windows, 1536-dim embeddings)
- [x] In-process 48→32 kHz resampling via rubato (FFT, 3s stride)
- [ ] Local audio capture via `cpal`
- [ ] SQLite detection log (rusqlite, WAL mode)

**Deliverable:** `cargo run` on an RPi, species detections in the terminal.

### Phase 2 -- API & Home Assistant

Expose detections over the network.

- [ ] axum REST API (`/detections`, `/status`)
- [ ] MQTT client with HA auto-discovery
- [ ] WebSocket live event stream
- [ ] Audio snippet saving (WAV, configurable retention)

**Deliverable:** Detections appear as HA sensor entities.

### Phase 3 -- Individual Recognition

"That's Barn Owl #1 again."

- [ ] Integrate Perch model (second consumer on ring buffer)
- [ ] rubato resampler for 48->32 kHz
- [ ] Embedding extraction pipeline
- [ ] `IndividualMatcher` with cosine similarity
- [ ] Enrolment API endpoint
- [ ] `individual` field in detection events

**Deliverable:** API returns individual IDs on detections.

### Phase 4 -- Multi-Station & Dashboard

Deploy to multiple stations, see everything in one place.

- [ ] Station-to-station MQTT federation
- [ ] Lightweight web dashboard (htmx or Leptos, no JS build chain)
- [ ] Spectrogram generation for detection review (`rustfft`)
- [ ] Detection export (CSV, JSON lines)
- [ ] Coral TPU support behind feature flag

**Deliverable:** Map-view dashboard showing all stations.

### Phase 5 -- Spatial Awareness

"The owl is 30 metres northwest."

- [ ] Multi-channel synchronised capture
- [ ] GCC-PHAT cross-correlation (`rustfft`)
- [ ] TDOA multilateration solver
- [ ] Location field in detection events
- [ ] Calibration tool for mic array geometry

**Deliverable:** Detections include estimated position.

## License

TBD
