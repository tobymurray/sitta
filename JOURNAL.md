# Sitta Development Journal

Decisions, insights, and lessons learned during development.

---

## 2026-04-15: Project Bootstrap

### Decision: Rust + Tokio async runtime
Chose Rust for the core engine targeting ARM64 SBCs (Raspberry Pi). Tokio provides
the async runtime for concurrent RTSP stream capture, inference fan-out, and future
API/MQTT tasks. The workspace is split into six crates for clean separation:
`sitta-audio`, `sitta-inference`, `sitta-store`, `sitta-api`, `sitta-spatial`, `sitta-bin`.

### Decision: ffmpeg subprocess for RTSP capture
Rather than pulling in a Rust RTSP client and codec libraries, we shell out to
ffmpeg and read raw `f32le` PCM from stdout. This mirrors BirdNET-GO's approach and
is codec-agnostic -- ffmpeg handles negotiation for whatever the camera/NVR speaks
(AAC, G.711, PCM, Opus, etc.). The tradeoff is a runtime dependency on ffmpeg, but
it's already installed on the target hardware.

**What worked:** Clean pipe-based design with `kill_on_drop(true)`, BufReader with
64KB buffer, and automatic reconnection on stream failure. Tested successfully
against a live RTSP stream (`rtsp://192.168.1.132:8554/north_feeder`) -- audio
chunks flowed correctly with sane RMS/dBFS values.

### Decision: Broadcast channel for audio fan-out
`tokio::sync::broadcast` with capacity 32 distributes `Arc<AudioChunk>` to all
consumers (inference, future storage, future API). This allows adding consumers
without modifying producers.

### Decision: Classifier trait abstraction
Designed a `Classifier` trait that both BirdNET and Perch can implement for species
identification. Originally the plan was "BirdNET for species, Perch for embeddings"
but the user clarified that Perch can also do species classification. The trait
returns `Vec<Classification>` with species name and confidence, making both models
interchangeable at the consumer level.

---

## 2026-04-15: BirdNET Model Loading -- The Hard Part

### Approach 1: ONNX via tf2onnx (FAILED)
**Plan:** Convert BirdNET v2.4 SavedModel to ONNX, load with `tract-onnx`.

**What happened:** Downloaded BirdNET v2.4 protobuf model from Zenodo. Attempted
conversion with `tf2onnx` (tried both opset 15 and 17). Failed with:
```
ValueError: make_sure failure: Current implementation of RFFT or FFT
only allows ComplexAbs as consumer not {'Cast'}
```

**Why it failed:** BirdNET's built-in spectrogram layer uses `tf.signal.stft` which
decomposes into RFFT operations. tf2onnx cannot convert these ops when they feed
into a `Cast` rather than `ComplexAbs`. This is a fundamental limitation of the
conversion tool, not a configuration issue.

**Lesson:** BirdNET bundles its own audio preprocessing (mel spectrogram) inside the
model graph. This makes the model self-contained but means the RFFT/spectral ops
block standard conversion paths.

### Approach 2: TFLite via `tflite` crate (FAILED)
**Plan:** Use the native TFLite model directly (`audio-model.tflite`, 50 MB) with
the `tflite` Rust crate.

**What happened:** The `tflite = "0.9.8"` crate vendors C++ TFLite code and builds
it via `cc`. Failed to compile with GCC 15:
- `__float128 is not supported on this target` (ruy math library)
- `no member named 'fwide' in the global namespace` (C++ standard library issue)
- bindgen assertion failures

**Why it failed:** The vendored C++ code hasn't been updated for GCC 15
compatibility. This is an upstream crate issue.

### Approach 3: `edgefirst-tflite` shared library (FAILED)
**Plan:** Use `edgefirst-tflite` which links against a pre-built
`libtensorflowlite.so`.

**What happened:** The crate expects `libtensorflowlite.so` on the system. There's
no packaged version for Arch Linux, and building TFLite from source (via Bazel) is
a heavyweight process.

**Why it failed:** Missing shared library dependency. Could be resolved by building
TFLite from source, but this adds significant build complexity.

### Insight: BirdNET model architecture
Examining the TF SavedModel revealed two signatures:
- **"basic"**: input `[1, 144000]` f32 waveform -> output `[6522]` species scores
- **"embeddings"**: input `[1, 144000]` f32 -> output `[1024]` embedding vector

The model's internal pipeline is: raw audio -> mel spectrogram (RFFT-based) -> CNN
backbone -> output head. The spectrogram layer is what blocks ONNX conversion.

### Approach 4: Split model -- Rust spectrogram + ONNX backbone (ABANDONED)
**Plan:** Compute the mel spectrogram in Rust (using `rustfft` + mel filterbank),
then export only the CNN backbone (post-spectrogram layers) to ONNX via tf2onnx
with `--inputs` to skip the preprocessing graph. Load in Rust with `tract-onnx`.

**Graph analysis results (2026-04-16):**

The model has TWO parallel mel spectrogram branches that get concatenated:

| Parameter      | MEL_SPEC1 | MEL_SPEC2 |
|----------------|-----------|-----------|
| frame_length   | 2048      | 1024      |
| frame_step     | 278       | 280       |
| fft_length     | 2048      | 1024      |
| mel_bands      | 96        | 96        |
| window         | Hann      | Hann      |
| mag_scaling    | 1.211     | 1.447     |

Each branch pipeline:
1. Input normalization: `(x - min) / (max - min)` → `(x - 0.5) * 2.0`
2. STFT with Hann window
3. Complex → magnitude squared (`Pow(x, 2.0)`)
4. Mel filterbank via Tensordot (→ 96 bands)
5. Power compression: `Pow(mel, 1/(1+exp(mag_scaling)))` (≈0.23 and ≈0.19)
6. ReverseV2, Transpose `[0,2,1]`, ExpandDims(-1)
7. Concatenate along axis 3 → `[batch, 96, 511, 2]`

Then: `BNORM_SPEC_NOQUANT` → `CONV_0(4×8, 2→24)` → EfficientNet-style
backbone (blocks 1-4 with SE attention) → `CLASS_DENSE_LAYER(1024→6522)`

**Why abandoned:** Cross-referencing with BirdNET-Analyzer source revealed that
the mel filterbank matrices are LEARNED weights, not standard mel triangular
filters. MEL_SPEC1 has shape `[96, 1025]` with only ~252 non-zero entries out of
98,400 -- extremely sparse and specific to the trained model. The power compression
exponents (0.2295, 0.1905) are also learned. Replicating this in Rust would require
extracting these exact weight matrices and reimplementing the entire non-standard
preprocessing pipeline. Too fragile and hard to validate.

### Insight: How BirdNET-Analyzer and BirdNET-GO actually work (2026-04-16)

**BirdNET-Analyzer (Python reference):** The Python code does NO spectrogram
preprocessing. It feeds raw `[1, 144000]` f32 audio directly to the model. All
spectrogram computation happens inside the TF/TFLite graph. The Python side only
handles audio loading, chunking, and post-processing of logits (sigmoid with
configurable sensitivity). Config: `SIG_FMIN=0`, `SIG_FMAX=15000`,
`SAMPLE_RATE=48000`, `SIG_LENGTH=3.0`.

**BirdNET-GO (Go reference):** Same approach -- feeds raw audio to TFLite. Uses
`go-tflite` (custom fork of TFLite C API bindings) with XNNPACK delegate. The
Go code is literally:
```go
copy(inputTensor.Float32s(), samples)
interpreter.Invoke()
copy(predictions, outputTensor.Float32s())
```
No spectrogram code in Go whatsoever. Also has an ONNX backend option.

**Key lesson:** Both reference implementations treat the model as a black box
(raw audio in → logits out). The split-model approach was fighting the design --
the spectrogram is integral to the model, not a separable preprocessing step.
The right path is to use TFLite directly, matching BirdNET-GO's strategy.

### Approach 5: TFLite via tract-tflite (FAILED)
**Plan:** Use `tract-tflite` (pure Rust TFLite loader) to load the .tflite model
directly. The `complex` feature flag seemed promising for RFFT support.

**What happened:** The `complex` feature flag is broken in 0.23.0-dev.3 (references
`num_complex` crate that isn't declared as a dependency, and `ComplexF64` variant
doesn't exist in tract-core). Without `complex`, the crate compiles but fails to
load the model: `Unsupported: SPLIT_V` -- it doesn't even get to the RFFT ops
before failing on a basic framing operation.

**Lesson:** tract-tflite has very incomplete op coverage. It can't handle BirdNET's
graph, which uses ops like `SPLIT_V`, `RFFT`, `COMPLEX_ABS` etc. that are common
in signal processing models but uncommon in the NLP/vision models tract targets.

### Approach 6: tflitec with pre-built library (FAILED -- bindgen)
**Plan:** Download pre-built `libtensorflowlite_c.so` from `tphakala/tflite_c`
(the same author as BirdNET-GO), use the `tflitec` Rust crate with
`TFLITEC_PREBUILT_PATH`.

**What happened:** The pre-built .so downloaded fine (v2.17.1, 4.7 MB). But
`tflitec` uses `bindgen 0.65.1` to generate Rust FFI bindings from C headers at
build time, and that bindgen version hits the same `__float128` assertion failure
on Arch Linux with GCC 15. Even with `BINDGEN_EXTRA_CLANG_ARGS="-D__float128=double"`,
a size assertion (4 vs 8) still fails.

Also tried building TFLite from source via CMake -- cmake configuration failed
due to FetchContent issues (protobuf clone failed, cmake_minimum_required
incompatibility with cmake 4.3.1).

### Approach 7: edgefirst-tflite with runtime dlopen (SUCCESS!)
**Plan:** Use `edgefirst-tflite` crate which loads `libtensorflowlite_c.so` at
runtime via `libloading` (dlopen/dlsym). No bindgen, no compile-time C headers.

**What worked:** Downloaded pre-built `libtensorflowlite_c.so` v2.17.1 from
`tphakala/tflite_c` (same lib BirdNET-GO uses). Set `LD_LIBRARY_PATH` to point
at it. The edgefirst-tflite crate compiled cleanly and loaded the model.

**Validation results:**
- Silence input: 0 detections (correct)
- Synthetic sine waves: detected "Siren" at 0.278 confidence (plausible)
- Performance: **44ms per inference** in release mode on x86_64

**Architecture:** Mutex-wrapped interpreter for thread safety. Library and Model
are Box::leak'd for 'static lifetime (they live for the process's duration).
The `Classifier` trait uses `&self` but interpreter needs `&mut self` for invoke(),
so Mutex provides interior mutability.

**Key insight:** The critical difference between edgefirst-tflite and tflitec is
that edgefirst-tflite does runtime symbol loading (libloading/dlopen) instead of
compile-time bindgen. This completely sidesteps the GCC 15 / bindgen 0.65
incompatibility that killed approaches 2 and 6.

### Insight: Python environment matters
Initial `pip install tensorflow` failed because the desktop machine had Python 3.14
(Arch Linux rolling release), and TensorFlow doesn't support 3.14 yet. Fixed by
installing Python 3.12 via pyenv.

### Insight: Zenodo download redirects
First attempt to download the BirdNET model via `curl` to a Zenodo URL returned an
HTML redirect page instead of the model file. Fixed by using the Zenodo API endpoint
(`/api/records/15050749/files/...`) which provides direct download links.

---

## Status

### What's working
- Workspace skeleton with all six crates
- RTSP audio capture via ffmpeg subprocess with reconnection
- Audio chunking, broadcast fan-out, graceful shutdown
- Classifier trait and BirdNET module structure (code compiles with tract-onnx)
- Config system with TOML deserialization
- Live-tested against real RTSP stream

### What's working (updated)
- **BirdNET inference is working!** via edgefirst-tflite + pre-built TFLite C lib
- 44ms per inference on x86_64, well within the 3s chunk window
- Full pipeline: RTSP → ffmpeg → PCM chunks → TFLite inference → detections

### Deviations from plan
- Originally planned tract-onnx with a full ONNX model. BirdNET's RFFT ops block
  tf2onnx conversion. Pivoted through split-model (abandoned due to learned
  filterbanks), now pursuing TFLite-native path matching BirdNET-GO's architecture.
- The dual-spectrogram with learned mel filterbanks was a key discovery -- it means
  the model MUST be run as a black box (raw audio in, logits out), exactly as
  BirdNET-Analyzer and BirdNET-GO do. Splitting was fighting the design.
- BirdNET-GO's approach (TFLite C API + XNNPACK delegate) is the proven path. The
  challenge is finding a Rust binding that compiles on modern toolchains.

---

## 2026-04-17: Switch inference backend to birdnet-onnx

### Decision: Replace edgefirst-tflite with birdnet-onnx

After landing on `edgefirst-tflite` as the working TFLite backend, discovered
`birdnet-onnx` — a crate by tphakala (the BirdNET-Go author) that wraps ONNX
Runtime with a purpose-built API for BirdNET-family models.

**Why switch:** `edgefirst-tflite` required runtime `dlopen` of a pre-built
`libtensorflowlite_c.so`, plus `unsafe impl Send/Sync`, a `Mutex<Interpreter>`,
`Box::leak` for 'static lifetimes, manual label parsing, and a manual sigmoid
implementation. `birdnet-onnx` handles all of this internally.

**What it gives us:**
- Builder pattern: `Classifier::builder().model_path(...).top_k(...).build()`
- Auto-detects model type (BirdNET v2.4/v3.0, Perch v2, BSG Finland)
- Thread-safe via internal `Arc` — no Mutex, no unsafe
- Labels parsed internally from the labels file
- Sigmoid applied internally — no `sigmoid_sensitivity` config knob
- `top_k` filtering built in
- `PredictionResult.embeddings` field — auto-populated for v3.0/Perch models
- ONNX Runtime bundled at build time (or `load-dynamic` feature for dlopen)
- Optional CUDA, TensorRT, CoreML, ArmNN, XNNPACK execution providers

**What simplified in birdnet.rs:** ~183 lines → ~120 lines. Removed:
- `unsafe impl Send/Sync` (birdnet-onnx is internally `Arc`-based)
- `Mutex<Interpreter>` (no mutable state to protect)
- `Box::leak` for Library and Model lifetimes
- `load_labels()` (handled internally)
- `sigmoid()` (handled internally)
- Hardcoded `SAMPLE_RATE`/`WINDOW_SAMPLES` constants (read from `config()`)

**Config change:** `sigmoid_sensitivity: f32` removed; `top_k: usize` added
(default 10). The sigmoid sensitivity was always 1.0 in practice — birdnet-onnx
matches BirdNET-Go's default sigmoid behaviour.

**Trait addition:** Added `classify_with_embeddings()` as a default method on the
`Classifier` trait, returning `Option<Vec<f32>>` alongside detections. The default
returns `None`; `BirdNet` overrides it to pass through `PredictionResult.embeddings`.
This prepares for Phase 3 (Perch individual identification) without breaking any
existing consumers.

**Builder path quirk:** `ClassifierBuilder::model_path` and `labels_path` take
`impl Into<String>`, not `&Path`, so paths are converted via
`path.to_string_lossy().into_owned()`. Straightforward but worth noting.
