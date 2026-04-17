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

### Approach 4: Split model -- Rust spectrogram + ONNX backbone (IN PROGRESS)
**Plan:** Compute the mel spectrogram in Rust (using `rustfft` + mel filterbank),
then export only the CNN backbone (post-spectrogram layers) to ONNX via tf2onnx
with `--inputs` to skip the preprocessing graph. Load in Rust with `tract-onnx`.

**Strategy:** Use tf2onnx but tell it to ignore the preprocessing layers. Freeze
the SavedModel graph (variables → constants), find the tensor name at the
concatenation point (where the two mel spectrograms merge), and use `--inputs`
to start the ONNX graph there. The Rust side computes mel spectrograms natively
with `rustfft`, producing the same tensor the backbone expects.

**Why this wins:** Pure Rust binary, no C++ TFLite dependency, no shared libraries.

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
7. Concatenate along axis 3 → `[batch, H, 96, 2]`

Then: `BNORM_SPEC_NOQUANT` → `CONV_0(4×8, 2→24)` → EfficientNet-style
backbone (blocks 1-4 with SE attention) → `CLASS_DENSE_LAYER(1024→6522)`

The backbone from `concatenate/concat` onwards is pure Conv2D, BatchNorm,
DepthwiseConv2D, MatMul, Relu, Sigmoid, Mean, Add -- all ONNX-convertible.

**Next step:** Freeze the graph, identify the exact tensor name at the
concatenation point, export backbone to ONNX, validate against full model.

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

### What's blocked
- BirdNET inference: need to export CNN backbone to ONNX (split-model approach)
- Three direct approaches failed (full ONNX conversion, tflite crate, shared lib)
- Split-model approach in progress -- graph fully analyzed, parameters extracted

### Deviations from plan
- Originally planned tract-onnx with a full ONNX model. The BirdNET spectrogram
  layer (RFFT ops) blocks this path. Pivoted to split-model: Rust mel spectrogram
  + ONNX backbone.
- TFLite was the natural fallback (it's BirdNET's native format) but the Rust
  ecosystem for TFLite on modern toolchains (GCC 15) is broken.
- The dual-spectrogram architecture was a surprise -- BirdNET uses two mel
  spectrograms at different resolutions (2048/1024 FFT) concatenated as a
  2-channel image. This is more complex to replicate in Rust than a single
  spectrogram, but the parameters are now fully known.
