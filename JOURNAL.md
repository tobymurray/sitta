# Sitta Development Journal

Decisions, insights, and lessons learned during development.

---

## 2026-04-21: Upgraded BirdNET range-filter to MData_Model_V2

### Context: previous meta model was an unversioned third-party conversion

The birda model registry pointed `birdnet-v24-meta.onnx` at a third-party HuggingFace
conversion (`justinchuby/BirdNET-onnx/birdnet_data_model.onnx`) whose provenance was
unclear — specifically, whether it was derived from the v1 or v2 release of the BirdNET
meta/data model.

BirdNET-Go uses `BirdNET_GLOBAL_6K_V2.4_MData_Model_V2_FP16.tflite` (the v2 release).
The old `birdnet-v24-meta.onnx` was ~14 MB — matching FP16 size — while the v2 TFLite
source is ~14 MB. It's plausible the `justinchuby` file was v1 or an FP16-preserved
conversion of v2; either way, it was not authoritative.

### Action

Downloaded the official v2.4 release archive from the Cornell/Chemnitz distribution:

  http://tuc.cloud/index.php/s/886x39f5N3sdsAM/download/v2.4.zip

Extracted `V2.4/BirdNET_GLOBAL_6K_V2.4_MData_Model_V2_FP16.tflite` and converted to
ONNX using `tf2onnx` (tensorflow-cpu 2.21.0, tf2onnx 1.17.0, opset 17):

  python3 -m tf2onnx.convert \
    --tflite BirdNET_GLOBAL_6K_V2.4_MData_Model_V2_FP16.tflite \
    --output birdnet-v24-meta.onnx \
    --opset 17

The conversion upsampled FP16 weights to FP32 (standard tf2onnx behaviour), growing
the file from ~14 MB to ~29 MB. Input/output shapes are identical to the old file:
`[batch, 3]` → `[batch, 6522]`. The `birdnet-onnx` crate selects tensors by index
so the new tensor names (`serving_default_MNET_INPUT:0`, `StatefulPartitionedCall:0`)
are compatible.

The resulting file was installed to `~/.local/share/birda/models/birdnet-v24-meta.onnx`,
replacing the previous third-party conversion.

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

### Decision: Deterministic UUIDv5 for config-derived entities

Stations and audio sources need stable database IDs across restarts so that
INSERT OR REPLACE doesn't orphan foreign keys. Solution: `Uuid::new_v5` with
a fixed Sitta namespace UUID + the config-provided string (station ID, source
name). Same config always produces the same UUID.

UUIDv7 is still used for detection IDs where time-sortability matters and
each event is unique.

### Decision: PersistCtx pattern for consumer integration

Both BirdNET and Perch consumers need the same set of database handles and
caches. A `PersistCtx` struct bundles: `Database` (cheap `Arc` clone),
`label_cache` (model_id + label_index → label_db_id), `model_ids` (display
name → model_db_id), `source_ids` (source name → UUID), and `station_id`.
Cloned into each consumer closure.

Database errors are logged but don't halt the pipeline — a transient write
failure shouldn't stop inference on a headless edge device.

### Insight: SQLx infers INTEGER PRIMARY KEY as nullable

SQLx's `query!` macro infers `INTEGER PRIMARY KEY` columns as `Option<i64>`
because SQLite technically allows NULL rowids in some edge cases. Fix with the
`!` override in SELECT: `SELECT id AS "id!" FROM models`. This tells sqlx the
value is guaranteed non-null. Affects the `models` and `labels` tables (both
use INTEGER PK).

### Bug: INSERT OR REPLACE orphans foreign key references

`INSERT OR REPLACE` in SQLite works by DELETE + INSERT. If `PRAGMA
foreign_keys` is not fully active (it defaults to OFF and must be set per
connection before any transaction), the DELETE silently succeeds even when
child rows reference the parent. The child rows are orphaned — still in the
database but invisible to JOIN queries.

This caused all historical detections to disappear on restart: the station
row was deleted and re-inserted, orphaning every detection that referenced
it. The detections were still in the `detections` table but the `JOIN
stations` in every query excluded them.

**Fix:** Replace `INSERT OR REPLACE` with `INSERT ... ON CONFLICT(id) DO
UPDATE SET ...` which updates in place without ever deleting. This is the
correct upsert pattern for SQLite when foreign key relationships exist.

**Lesson:** Never use `INSERT OR REPLACE` on tables that are referenced by
foreign keys. It's a well-documented SQLite footgun — the DELETE step can
cascade or orphan depending on FK enforcement state.

---

## 2026-04-17: Audio clip saving, spectrograms, and detection review

### Decision: Save the analysis window, not a longer clip

BirdNET-Go saves 15-second clips (3s pre-buffer + detection + post) from a 120-second
ring buffer. After researching user feedback, the core use case is "hear what the model
heard" for false-positive triage. Saving the exact analysis window (3s for BirdNET, 5s
for Perch) is sufficient for this and avoids the memory overhead of a ring buffer on a
2GB Pi. A ring buffer for longer clips can be added later.

### Decision: Overlapping BirdNET windows with configurable stride

BirdNET-Go uses 3s windows with 1s stride (2s overlap) to avoid missing detections at
chunk boundaries. Sitta's BirdNET consumer was processing chunks 1:1 (no overlap).
Refactored both consumers to use the same sliding-window pattern:

| Consumer | Window | Stride | Overlap |
|----------|--------|--------|---------|
| BirdNET  | 3s (144k samples) | 1s (48k) | 2s |
| Perch    | 5s (240k samples) | 3s (144k) | 2s |

The BirdNET stride is configurable via `inference.birdnet.stride_seconds` (default 1.0).
Setting stride = chunk_seconds (3.0) disables overlap for CPU-constrained boards.

With 1s stride, BirdNET runs ~3x more inference per second. On a Pi 5, BirdNET inference
takes ~200ms per window, so 3 windows per 3s chunk = 600ms / 3s = ~20% CPU. Acceptable.

### Decision: Async snippet writer with bounded channel

Audio saving must not block inference. The snippet writer uses a bounded `mpsc` channel
(capacity 64) feeding a single background task. The task writes WAV files inside
`spawn_blocking` (SD card I/O must not block the async runtime). If the channel is full
(SD card saturated), jobs are dropped with a warning — never blocks.

**Atomic writes:** WAV data goes to `{path}.wav.tmp`, then `fs::rename` on success.
This prevents the API from serving partial files. The audio endpoint returns 503 +
`Retry-After: 1` if a `.tmp` file exists.

**File layout:** `clips/{YYYY-MM-DD}/{detection_id}.wav`. Using the detection UUID as
the filename avoids special-character issues in species names and makes DB lookups O(1).

### Decision: 16-bit PCM WAV, not f32

The audio pipeline uses f32 samples internally, but WAV files are written as 16-bit PCM.
This halves file size (3s @ 48kHz = ~282KB vs ~562KB) with negligible quality loss for
the review use case. The WAV writer is a dependency-free 60-line module in sitta-audio.

### Decision: Pure-Rust mel spectrograms (no sox dependency)

BirdNET-Go shells out to `sox` for spectrogram generation with an `ffmpeg` fallback.
Sitta uses `rustfft` + `image` crate instead — no external binary dependencies beyond
ffmpeg (which is already needed for RTSP).

Parameters: 512-point FFT, 256 hop (50% overlap), 80 mel bins (150 Hz – 15 kHz),
viridis-style colormap. Output: 800x200 PNG. On a Pi 5, generation takes <50ms including
PNG encoding.

**On-demand with disk cache:** Spectrograms are generated when first requested via the
API, then cached as `.png` alongside the `.wav` file. Aggressive `Cache-Control:
immutable` headers prevent re-requests.

### Decision: Detection review workflow

The `detection_reviews` table already existed in the schema. Added:
- `PUT /api/v1/detections/{id}/review` — mark as correct or false_positive
- `GET /api/v1/detections/{id}/review` — fetch review status
- `DELETE /api/v1/detections/{id}/review` — un-review

Dashboard integration: checkmark and X buttons on every detection card, plus keyboard
shortcuts (hover + c/f) for rapid bulk triage. This matches the quick-review workflow
that BirdNET-Go users explicitly requested (GitHub issue #2712).

Reviewed-as-correct clips are never deleted by the retention worker.

### Decision: Age + size retention policy

SD cards are small. Two retention strategies run hourly:
- **Age-based:** delete clips older than `retention_days` (default 30)
- **Size-based:** if total clip storage exceeds `max_disk_mb` (default 2GB), delete
  oldest unreviewed clips until under the limit

Both strategies skip clips reviewed as "correct" — they are effectively pinned.
Spectrograms are deleted alongside their WAV files.

### Insight: Perch consumer was passing the wrong audio to persist

The Perch consumer accumulated chunks in a buffer, extracted 5s windows for inference,
but passed the *last received 3s chunk* (not the 5s window) to `persist_detections()`.
This meant snippet saving would have captured only 3s of a 5s analysis. Fixed by
constructing a synthetic `AudioChunk` from the full 5s window (at 48kHz, before
resampling) and passing that to persist instead.

### Insight: BirdNET-Go audio clips are a core feature, not nice-to-have

Research into BirdNET-Go's GitHub issues and community revealed that audio clip saving is
the most important user-facing feature. When it broke in v0.6.3, multiple users filed
bugs within hours and the maintainer shipped a P0 hotfix. Users do daily bulk false-positive
triage by listening to clips — this is the primary workflow, not an optional extra.
Spectrograms are a strong supporting feature (users develop visual pattern recognition),
and the review workflow is what makes it actionable.

---

## 2026-04-19: Rarity Scoring

### Decision: Three-axis rarity score computed at detection time

Every detection now gets a rarity score breaking down three dimensions:

1. **Local rarity** — novelty at this station: first-ever, first-of-season (meteorological,
   hemisphere-aware), first-of-week, first-of-day, days since last detection, prior count.
2. **Regional rarity** — inverted BirdNET meta-model location score. The range filter already
   computes per-species occurrence probabilities for the station's lat/lon + today's date;
   we now cache the raw scores alongside the allowed set and expose them for rarity.
3. **Temporal rarity** — how unusual the detection hour is vs. the species' historical hourly
   profile. A nocturnal detection of a diurnal species scores high.

The composite score (0.0=common, 1.0=extremely rare) weights local 40%, regional 35%,
temporal 25%. Stored in a new `detection_rarity` table, indexed by score for efficient
"show me the most unusual detections" queries.

### Decision: Score at insert time, not read time

Rarity is computed during `persist_detections()` and stored alongside the detection. This
keeps the read path trivial (one extra JOIN) and means the score reflects the state of knowledge
*at the time of detection* — which is the semantically correct interpretation. If a species
has never been seen and then appears, that first detection should always be scored as first-ever,
even after hundreds more follow.

### Decision: Extend RangeFilter to cache per-species scores

The existing `RangeFilter` cached only a `HashSet<String>` of allowed species. Changed to
also cache a `HashMap<String, f32>` of raw location scores. The new `score_for()` method
lets any caller look up the meta-model's occurrence probability for a species. This is
useful beyond rarity — it could feed into future confidence calibration or UI features.

### Enhancement: Species detail page with seasonality and today-likelihood

Extended the species insights API and detail page with:

- **Monthly distribution** — 12-month bar chart showing seasonal patterns (year-round
  resident vs seasonal visitor vs migrant).
- **Today likelihood** — composite score combining range model probability, monthly
  frequency, hourly activity, and detection consistency. Displayed as a prominent badge.
- **Data sufficiency** — amber callout panel that tells users *what's missing*: "Only 3
  of 12 months have data", "Observation window is only 12 days", etc. This guides users
  toward collecting more useful data rather than hiding uncertainty.
- **Notable detections** — panel highlighting the highest-rarity detections (first-ever,
  first-of-season) with links to the detection detail page.
- **Rarity badges** — detection cards now show "First ever", "First of season", "First
  this week", "First today", and "Rare" badges derived from the per-detection rarity score.

Weather/temperature correlation is a natural next step but requires an external data source
(weather API or local sensor). Flagged as future work — the data sufficiency framework
is already in place to add "Need weather data for correlation" once a source is available.

---

## 2026-04-20: Effort Tracking

### Decision: Automatic session tracking via broadcast subscriber

Without effort data, detection counts are meaningless — "2 detections in 6 hours" is very
different from "2 detections in 5 minutes." Effort tracking records *when each audio source
was actually recording*, turning a species list into occupancy data suitable for publication.

**Architecture:** A dedicated background task subscribes to the existing audio broadcast
channel and tracks per-source "liveness." When the first chunk from a source arrives (or
arrives after a gap), a session opens. When no chunks arrive for a configurable timeout
(2× chunk duration + 5s buffer), the session closes with reason `"gap"`. On shutdown,
all sessions close with reason `"shutdown"`.

**Why broadcast subscriber, not source-internal hooks:** This approach requires zero
modifications to the RTSP, Remote, or future Local source implementations. It works for
any source type automatically. The broadcast channel is already the fan-out mechanism for
audio — effort tracking is just another consumer, like BirdNET or Perch inference.

**Crash safety:** On startup, any sessions left open by a prior unclean shutdown are
closed with reason `"startup_cleanup"`. This prevents stale open-ended sessions from
inflating effort calculations.

**Chunk counter batching:** Rather than writing to the database on every chunk (every 3s
per source), chunk counts are accumulated in memory and flushed every 10 chunks. This
reduces SQLite write pressure on SD cards while keeping the count reasonably current.

### Schema: source_sessions table

```sql
source_sessions (
    id              BLOB PRIMARY KEY,   -- UUIDv7
    source_id       BLOB NOT NULL REFERENCES audio_sources(id),
    started_at      INTEGER NOT NULL,   -- epoch ms
    ended_at        INTEGER,            -- NULL while active
    end_reason      TEXT,               -- 'gap', 'shutdown', 'removed', 'startup_cleanup'
    chunks_received INTEGER NOT NULL DEFAULT 0
)
```

Indexed on `(source_id, started_at)` for per-source queries and `(started_at, ended_at)`
for time-range effort summaries.

### API: GET /api/v1/effort

Returns effort data for any time window (default: last 24h):

- `total_recording_seconds` — total audio captured across all sources
- `overall_coverage` — fraction of the time window covered (0.0–1.0)
- Per-source breakdown: `total_seconds`, `session_count`, `coverage`
- `active_sessions` — currently-recording sources with duration and chunk count

The effort summary query clamps sessions to the requested window boundaries, so a session
that started before the window or is still active contributes only the overlapping portion.

### Enhancement: Status endpoint

`GET /api/v1/status` now includes `active_sources` — the list of source names currently
receiving audio, derived from open sessions.

### Insight: Gap timeout tuning

The gap timeout (time without chunks before a session closes) is set to `2 × chunk_seconds
+ 5s`. For the default 3s chunks, this is 11s. This accounts for: one missed chunk (RTSP
hiccup), processing delays, and a small buffer. Short enough to accurately reflect real
disconnects, long enough to avoid false session splits from transient network blips.

---

## 2026-04-20: Species list confidence filtering and range filter visibility

### Bug: Species list hid low-confidence detections

The species list API (`GET /api/v1/species`) was filtering by `display_min_confidence`
(default 0.65). This meant species that were only detected at lower confidence levels
(e.g., 0.30–0.64) were completely invisible on the species page — even though the
detections existed in the database. The detection *list* filtering makes sense (you don't
want to scroll through noise), but the species *index* should show every species that has
any detection at all.

**Fix:** Removed the confidence floor from `species_summary()`. The species list now shows
all species with at least one detection in the time window, regardless of confidence. The
detection list and dashboard still respect `display_min_confidence`.

### Insight: Range filter silently drops detections

The BirdNET range filter (`meta_model_path` + station lat/lon) calls `birdnet_onnx`'s
`predict()` which returns only species whose location score >= `meta_threshold` (default
0.01). Sitta's `RangeFilter::filter()` then drops any detection whose species isn't in
that set. If a species IS present at the station but the meta-model doesn't expect it
(score < 0.01), the detection is silently discarded before it ever reaches the database.

This was the most likely explanation for why BirdNET-GO was detecting a species (Barred Owl
at 75% confidence) that Sitta was not: BirdNET-GO either had the range filter disabled or
configured more permissively.

**Mitigation:** Added `DEBUG`-level logging when the range filter drops a detection,
including species name, scientific name, and confidence. Run with `RUST_LOG=sitta=debug`
to see which species are being filtered. If a species is being incorrectly dropped,
options are:
1. Add its eBird species code to `[inference.birdnet] force_allow`
2. Lower `meta_threshold` (e.g., to 0.001)
3. Remove `meta_model_path` to disable the range filter entirely
