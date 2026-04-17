# Sitta Vision

Sitta is a Rust bioacoustics engine for local-network bird identification.
It is inspired by [BirdNET-Go](https://github.com/tphakala/birdnet-go) and
shares the `birdnet-onnx` crate with it (both by tphakala).

## Relationship to BirdNET-Go

BirdNET-Go is the reference implementation. It is a mature, production-ready
Go application with RTSP capture, BirdNET inference, MQTT/Home Assistant
integration, a web dashboard, SQLite detection log, and species filtering.

Sitta targets the same use case — local bird identification on ARM64 SBCs —
but is implemented in Rust and extends the model support beyond BirdNET v2.4.

### Feature parity targets (from BirdNET-Go)

| Feature | BirdNET-Go | Sitta | Phase |
|---|---|---|---|
| RTSP audio capture via ffmpeg | Yes | Yes | 1 |
| BirdNET inference (v2.4) | Yes | Yes | 1 |
| Configurable confidence threshold | Yes | Yes | 1 |
| SQLite detection log | Yes | Planned | 2 |
| MQTT / HA auto-discovery | Yes | Planned | 2 |
| REST API | Yes | Planned | 2 |
| Web dashboard | Yes | Planned | 4 |
| Species filtering by location/date | Yes | Planned | 4 |
| Audio snippet saving | Yes | Planned | 2 |
| Coral TPU support | Yes | Planned | 4 |

### Sitta differentiators

**Multi-model support.** Sitta's `Classifier` trait is model-agnostic. BirdNET
v3.0, Google Perch v2, and BSG Finland are all first-class. Perch produces
1280-dimensional embeddings for individual animal identification — recognising
specific animals across sessions, not just species. BirdNET-Go does not support
individual identification.

**Rust.** Single static binary, no runtime except ffmpeg. Memory safety without
garbage collection. Straightforward ARM64 cross-compilation (`cross`). No JVM,
no Python interpreter, no Go runtime.

**Shared inference crate.** `birdnet-onnx` is maintained by tphakala (BirdNET-Go
author). Sitta benefits from the same model support improvements that BirdNET-Go
gets, without diverging on model loading logic.

### Architectural differences

BirdNET-Go uses TFLite C API (via a custom fork of go-tflite) with XNNPACK
delegate. Sitta uses ONNX Runtime via birdnet-onnx, which also supports CUDA,
TensorRT, CoreML, and ArmNN. The ONNX path avoids the TFLite C shared library
dependency that caused the 7-approach odyssey documented in JOURNAL.md.

BirdNET-Go has a monolithic architecture. Sitta uses a Rust workspace with six
crates (`sitta-audio`, `sitta-inference`, `sitta-store`, `sitta-api`,
`sitta-spatial`, `sitta-bin`) for clean boundaries and independent compilation.

## Future: Individual Identification (Phase 3)

Sitta's key extension beyond BirdNET-Go's feature set:

1. **Enrolment.** User labels a detection as "Barn Owl #1." The Perch embedding
   vector is stored in `sitta-store`.
2. **Matching.** New Perch embeddings are compared against known individuals via
   cosine similarity. Threshold: configurable, default 0.85.
3. **Brute-force search.** Dozens to low hundreds of individuals don't need a
   vector database — in-process cosine search is fast enough.
4. **Detection events** carry an `individual` field when matched.

This is the primary reason for the multi-model `Classifier` trait abstraction.

## Future: Spatial Awareness (Phase 5)

Four time-synchronised microphones + GCC-PHAT cross-correlation + TDOA
multilateration → estimated (x, y) position of a calling bird. Detection events
carry a `location` field. BirdNET-Go has no equivalent.
