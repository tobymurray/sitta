# BirdNET Inference Backend Analysis

Evaluation of approaches for running BirdNET v2.4 inference in Rust, targeting
x86_64 desktops and ARM64 SBCs (Raspberry Pi 4/5, Orange Pi 5 Pro).

## Winner: `birdnet-onnx` crate + ONNX Runtime

**Crate:** [`birdnet-onnx`](https://crates.io/crates/birdnet-onnx) (v2.0.0-rc.13)
by tphakala (author of BirdNET-GO, 1000+ stars).

**Model:** [`birdnet.onnx`](https://huggingface.co/justinchuby/BirdNET-onnx) on
HuggingFace. Converted by justinchuby from the official TFLite model. Accepts raw
f32 audio (144,000 samples @ 48 kHz) and outputs 6,522 species logits -- the
STFT/mel spectrogram ops are embedded in the ONNX graph.

### Why this wins

| Factor | birdnet-onnx (ONNX Runtime) | edgefirst-tflite (TFLite C API) |
|--------|-----------------------------|---------------------------------|
| **Compile** | Clean, no native deps at build time | Clean (runtime dlopen) |
| **Runtime lib** | `ort` auto-downloads on x86_64 | Manual `libtensorflowlite_c.so` install |
| **ARM64 support** | Official ORT aarch64 builds; `birda` has `build:linux-arm64` task | Pre-built .so from tphakala/tflite_c |
| **Model support** | BirdNET v2.4, v3.0, Perch v2, BSG Finland | BirdNET v2.4 only (our wrapper) |
| **GPU** | CUDA, TensorRT via ort execution providers | Not in pre-built .so |
| **API** | Builder pattern, batch inference, range filtering, thread-safe | Our own basic Classifier trait |
| **Label handling** | Built-in with multi-language support | Manual parsing |
| **Sigmoid/postproc** | Built-in with configurable sensitivity | Our own code |
| **Maintenance** | 148 commits, actively maintained, same author as BirdNET-GO | Our own code |
| **Crates.io** | Published (`birdnet-onnx = "2.0"`) | Not published |

### Platform support

| Platform | Status | Notes |
|----------|--------|-------|
| x86_64 Linux (desktop) | **Works** | `ort` auto-downloads ONNX Runtime via `download-binaries` |
| aarch64 Linux (Pi 4/5) | **Works** | Use `load-dynamic` feature + manual `libonnxruntime.so` from [ORT releases](https://github.com/microsoft/onnxruntime/releases). `birda` (by same author) explicitly supports `linux-arm64`. BirdNET-GO runs on Pi 3B+ and up. |
| Orange Pi 5 Pro (RK3588) | **Works (CPU only)** | Cortex-A76/A55 cores run ORT CPU EP fine. The RK3588 NPU uses proprietary RKNN format -- not accessible via ONNX Runtime. Mali GPU also has no ORT EP. |

### Known consumers

- [**birda**](https://github.com/tphakala/birda) -- CLI tool for bird species
  detection by tphakala. Uses `birdnet-onnx 2.0.0-rc.13` with `load-dynamic`
  feature. Has explicit ARM64 Linux build support.
- [**birdnet-go**](https://github.com/tphakala/birdnet-go) -- The Go equivalent
  (1029 stars). Uses the same ONNX model. Proven on Pi 3B+/4/5.

### ARM64 deployment strategy

For Raspberry Pi / Orange Pi deployment:

1. Use `birdnet-onnx` with `ort`'s `load-dynamic` feature (don't rely on
   `download-binaries` for ARM).
2. Download `libonnxruntime.so` from Microsoft's official
   [aarch64 release tarballs](https://github.com/microsoft/onnxruntime/releases)
   (e.g., `onnxruntime-linux-aarch64-1.23.0.tgz`).
3. Install the .so to `/usr/local/lib` or set `ORT_DYLIB_PATH`.
4. For XNNPACK acceleration (ARM NEON optimized), build ORT from source with
   `--use_xnnpack`. This provides 2-3x speedup on ARM for conv-heavy models.

### RK3588 NPU note

The Orange Pi 5 Pro's 6 TOPS NPU cannot be used with ONNX Runtime. It requires
Rockchip's RKNN format (convert via `rknn-toolkit2`, opset <= 16, kernel 5.10).
This would be a separate inference path -- not worth pursuing initially since
CPU inference on the A76 cores should be fast enough (~100-200ms per 3s chunk
based on BirdNET-GO's Pi 4 benchmarks).

---

## Rejected alternatives

### 1. Full ONNX conversion via tf2onnx
BirdNET's STFT layer uses RFFT ops that tf2onnx can't convert (`Cast` consumer
instead of `ComplexAbs`). Dead end.

### 2. `tflite` crate (vendored C++)
Vendors TFLite C++ and builds via `cc`. Fails with GCC 15 (`__float128`,
`fwide`, bindgen assertions). Upstream crate issue.

### 3. `edgefirst-tflite` (runtime dlopen)
**Works**, but requires manual .so management and we maintain our own wrapper
code. No Perch support, no range filtering, no batch inference.

### 4. `tflitec` (C API + bindgen)
Fails at build time due to bindgen 0.65 incompatibility with GCC 15 headers.
Same `__float128` / size assertion issue as the `tflite` crate.

### 5. `tract-tflite` (pure Rust)
Fails to load the model: `Unsupported: SPLIT_V`. Incomplete op coverage --
can't handle BirdNET's signal processing ops.

### 6. Split model (Rust spectrogram + ONNX backbone)
Abandoned after discovering BirdNET uses **learned** mel filterbank weights
(not standard mel triangular filters). The filterbank matrices are sparse,
non-standard, and baked into the model. Splitting would require extracting and
reimplementing the entire non-standard preprocessing.
