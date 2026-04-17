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

### Where does the BirdNET ONNX model come from?

Zenodo only distributes BirdNET v2.4 as TFLite/Keras/Protobuf — no ONNX. And
tf2onnx fails on the RFFT spectrogram ops (Approach 1 in this journal). The
solution is `justinchuby/BirdNET-onnx` on HuggingFace — Justin Chu converted
the model using NVIDIA Nsight DL Designer (not tf2onnx), which successfully
handles the spectrogram ops. The recommended file is `birdnet.onnx`.

The easiest way to get it is [`birda`](https://github.com/tphakala/birda), a
CLI model manager by tphakala (the birdnet-onnx author):
```bash
birda models install birdnet-v24
```
This handles download of both the ONNX model and labels file.

---

## 2026-04-17: Perch v2 integration

### Decision: Perch as a second independent consumer, in-process resampling

Perch v2 expects 32 kHz / 5s windows (160,000 samples). The RTSP pipeline produces
48 kHz / 3s chunks (144,000 samples). Rather than changing the audio capture pipeline,
Perch gets its own consumer task that buffers incoming 48 kHz chunks and resamples
in-process.

**Resampling approach:** `rubato` v2.0 `Fft` resampler (48000→32000 Hz, `FixedSync::Both`,
1024-sample chunk hint). `FixedSync::Both` is the right mode for offline/batch use:
it adjusts the internal chunk size to fit the exact ratio (3:2), avoiding fractional-sample
accumulation. The `process_all_into_buffer()` method handles the full 240k-sample window
in one call, including output delay trimming.

**Buffer/stride design:**
- Buffer accumulates raw 48 kHz samples from the broadcast channel
- When ≥240,000 samples: resample → 160,000 @ 32 kHz → Perch inference
- Drain 144,000 samples (3s / one chunk) → 2s overlap between consecutive windows
- On channel lag: clear buffer (avoids processing stale audio)

**rubato 2.0 vs 0.x:** The 2.0 API uses `AudioAdapter` traits for I/O instead of
`Vec<Vec<f32>>`. `InterleavedSlice` from `rubato::audioadapter_buffers::direct`
wraps a `&[f32]` for mono audio. Re-exported by rubato — no extra dependency needed.
`SincFixedIn` no longer exists; it's now `Fft::new(in_rate, out_rate, ...)`.

**Perch model characteristics (verified):**
- Auto-detected as `PerchV2` by birdnet-onnx (160k input, 4 outputs)
- Always returns `Some(Vec<f32>)` embeddings — 1536 dimensions
- Softmax activation (not sigmoid like BirdNET)
- 65ms per inference on x86_64 (release mode)
- Labels file is CSV format (not .txt) — birdnet-onnx auto-detects

**Model acquisition:** `birda models install perch-v2` downloads to
`~/.local/share/birda/models/perch-v2.onnx` + `perch-v2.csv`.

**Embeddings:** 1536-dim vectors logged at `DEBUG` level only. Storage deferred to
Phase 3 (sitta-store not yet implemented).

---

## 2026-04-17: eBird taxonomy integration

### Decision: sitta-taxonomy crate for common-name resolution

Perch v2 labels are bare scientific names in underscore form (`Tyto_alba`, `Turdus_migratorius`).
The existing `parse_species` split-on-`_` logic gives nonsense results: `"Tyto_alba"` produces
`scientific="Tyto"`, `common="alba"`. We need the eBird taxonomy to map scientific names to
English common names and species codes.

**New crate: `sitta-taxonomy`**
Wraps the eBird taxonomy CSV (download: `https://api.ebird.org/v2/ref/taxonomy/ebird?fmt=csv`).
Key columns used: `SCI_NAME`, `PRIMARY_COM_NAME`, `SPECIES_CODE`. Lookup key is the scientific
name normalized to lowercase with underscores replaced by spaces. The same normalization handles
both Perch labels (`Tyto_alba`) and BirdNET labels (`Tyto alba`) transparently.

**Label parsing logic (updated):**
1. Try the whole label normalized as a scientific name against the taxonomy (handles Perch)
2. If found: use taxonomy's canonical name + common name + species code
3. If not found: split on first `_` for BirdNET format `"Scientific Name_Common Name"`,
   then still try a taxonomy lookup on just the scientific part to get the species code

**`Species` struct change:** Added `taxon_code: Option<String>` (eBird species code,
e.g., `"barowl1"`). Present when taxonomy is loaded, `None` otherwise. Used in detection
log output and will feed the future MQTT schema's `taxon_id` field.

**Config:** Optional `[taxonomy]` section with `ebird_path`. If absent, all taxonomy
enrichment is skipped — existing behavior preserved. Both BirdNET and Perch classifiers
accept the same `Option<Arc<EbirdTaxonomy>>`.

**Taxonomy loading:** Load once at startup, wrap in `Arc`, clone the `Arc` cheaply to each
classifier. The `HashMap` is immutable after construction so no locking is needed.

---

## 2026-04-17: Geographic/seasonal range filter

### Decision: BirdNET meta-model via birdnet-onnx RangeFilter

`birda models install birdnet-v24` already downloads `birdnet-v24-meta.onnx` alongside
the main model. `birdnet-onnx` already has a `RangeFilter` type wrapping it. No new
dependencies needed.

**How it works:** `RangeFilter::predict(lat, lon, month, day)` runs a tiny ONNX session
with input `[lat, lon, week]` (48-week BirdNET calendar) and outputs a probability score
for each of the 6522 species. Species below the threshold are filtered from detections.

**Architecture:** `RangeFilter` lives in `sitta-inference::rangefilter` and wraps
`birdnet_onnx::RangeFilter`. It holds a date-keyed `Mutex<Option<Cached>>` where
the cached value is an `Arc<HashSet<usize>>` of allowed label indices. On the first
call each calendar day the meta-model runs (CPU-bound, fast); subsequent calls for the
same day are O(n) HashSet lookups — no ONNX session touch.

**Why label indices, not species strings:** `Classification.label_index` (from
`birdnet_onnx::Prediction.index`) maps directly to `LocationScore.index`. Filtering
by index avoids string comparisons and is unaffected by label format differences.

**Where it's applied:** Inside `handle_chunk`'s `spawn_blocking` closure, immediately
after `Classifier::classify()` returns. The filter `Arc` is cloned cheaply for each
inference task. Perch does NOT get the range filter — its 14,795-species label space is
different from BirdNET's 6,522.

**Key constraint:** `BirdNet` must not be erased to `Arc<dyn Classifier>` until AFTER
the `RangeFilter` is built, because the filter needs `model.labels()` (the raw label
slice from the ONNX session). `load_birdnet()` returns both together before type erasure.

**Config:** `[station] latitude`/`longitude` + `[inference.birdnet] meta_model_path` /
`meta_threshold` (default 0.01). Warning logged and filter disabled if lat/lon are
missing when meta_model_path is set.

**Real-world result (2026-04-17, Melbourne -37.81, 144.96):** 154 species allowed out
of 6,522 — dramatically reduces false positives from species that simply don't occur here.

---

## 2026-04-17: SQLite persistence — schema design and library choice

### Decision: SQLite schema for sitta-store

Designed a 10-table SQLite schema (`sitta-store/schema.sql`) informed by
BirdNET-Pi and BirdNET-Go's migration pain points. Key design calls:

- **INTEGER PKs for dimension tables** (`models`, `labels`), **UUIDv7 BLOB(16)
  PKs for entity/event tables** (detections, individuals, etc.). Labels table
  has ~21,000 rows referenced from every detection — 4-byte INTEGER FK vs
  16-byte BLOB saves meaningful space on SD card storage.
- **Single `detected_at` INTEGER** (Unix ms) instead of separate Date/Time
  string columns. BirdNET-Pi's dual-column design complicated every range
  query; BirdNET-Go v2 fixed this.
- **Labels are per-model** (`UNIQUE(model_id, label_index)`) because the same
  scientific name appears at different tensor positions across BirdNET and
  Perch. Non-species labels (noise, environment) use `scientific_name = NULL`.
- **Top-1 prediction inline on detections**, secondary predictions in
  `detection_predictions` with a rank column. `WITHOUT ROWID` on predictions
  since the composite PK `(detection_id, rank)` is the only access pattern.
- **Nullable `location_x`/`location_y`** on detections for Phase 5 TDOA —
  avoids a sparse join table for the ~1% of detections that will have
  location.
- **`metadata` JSON blob** for extensible per-detection diagnostics (noise
  floor, peak freq, inference time) that won't be filtered on.
- **`ON DELETE CASCADE`** from detections to predictions, embeddings, matches,
  and reviews. Stations and models use default RESTRICT to prevent accidental
  mass deletion.

Full implementation plan in `STORE_IMPLEMENTATION_PLAN.md`.

### Decision: SQLx over rusqlite

Initially planned `rusqlite` (raw SQL, minimal abstraction). Switched to
`sqlx` for **compile-time query checking**.

**Why:** The SQL boundary (column names, types, nullability) is where bugs
historically hide in projects like this. A renamed column or mismatched type
silently compiles with string-based SQL and only fails at runtime — possibly
on a headless Pi in the field. `sqlx::query!` macros check every query
against the real schema at compile time.

**What we lose:** Nothing material. The raw-SQL philosophy is preserved —
`sqlx::query!` is still handwritten SQL, not an ORM query builder.
`sqlx::raw_sql()` handles PRAGMAs and DDL.

**Cross-compilation concern (resolved):** `sqlx::query!` needs a database at
compile time. Offline mode (`cargo sqlx prepare`) caches query metadata in
`.sqlx/` (committed to repo). CI and aarch64 cross-builds use
`SQLX_OFFLINE=true` — no database needed.

**Architectural simplification:** `SqlitePool` is `Clone + Send + Sync` and
serializes writes internally. The dedicated writer thread + mpsc channel
pattern from the rusqlite plan is replaced by sharing the pool across async
tasks directly.

### Insight: SQLx infers INTEGER PRIMARY KEY as nullable

SQLx's `query!` macro infers `INTEGER PRIMARY KEY` columns as `Option<i64>`
because SQLite technically allows NULL rowids in some edge cases. Fix with the
`!` override in SELECT: `SELECT id AS "id!" FROM models`. This tells sqlx the
value is guaranteed non-null. Affects the `models` and `labels` tables (both
use INTEGER PK).
