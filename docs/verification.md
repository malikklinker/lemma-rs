# Reproducible verification

## 2026-10-28 integration pass

The PR preparation pass passed formatting, Clippy with warnings denied, 31 Rust
library tests, three delivery tests, six evaluation tests, six host tests, the
immutability doctest, TypeScript checking, and all 52 actual Node cases. Node
tests ran with subprocess access; restricted file-wrapper output was not counted.

The opt-in GPU suite passed on Vulkan llvmpipe (Mesa 26.1.8 / LLVM 21.1.8,
software CPU, 4x MSAA), including a new two-renderer ownership contract: foreign
frame capture returns an error, and reusing one capture object across equal-sized
frames from different renderers replaces device-owned staging memory safely.
Other new regressions cover marker projection overflow with stable IDs,
serialization loss with node/property diagnostics, and invalid first export
samples leaving outputs/encoders untouched.

Fresh artifacts: visually inspected `out/pr-review-0240.png` (640x360), and
`out/pr-review-short.mp4`. The latter was verified with FFprobe: H.264, 320x180,
12 frames, `30000/1001` fps, 0.400400 seconds. See the
[development log](development-log.md) for scope and limitations. Hardware
performance and native presentation were not remeasured in this pass.

## Earlier acceptance-scene evidence

Measured on 2026-10-26. These checks describe the implemented acceptance scene,
separately from the broader proposed APIs in the design documents. All generated
images, videos, logs, and reports remain in ignored `out/`.

## Completion against the initial target

| Slice | Implemented result | Evidence |
| --- | --- | --- |
| Frame kernel | Explicit-sample curve and repeated opaque sphere markers, one selected device/camera, PNG capture. | Inspected 1280×720 frames 0, 240, 479; stroke and retained-frame GPU contracts. |
| Field/depth | Stable grid topology, scalar palette, animated displacement/model, shared opaque depth, cached uploads. | Occlusion/lighting contracts; model/camera/palette edits preserve numerical identities and upload counts. |
| Mixed graphics | Fixed equation/legend, projected label with GPU `hide`/`always` visibility, world text plane. | Linear alpha/sRGB, exact legend, world-plane depth and transformed-attachment contracts; inspected PNGs. |
| Authoring loop | TSX constructs version 1 scenes; persistent validation host and watched native preview retain valid scenes after failure. | 19 Node cases, whole-reference parity, actual watch recovery, native X11 reload test. |
| Numerical data | Synthetic field and validated binary saved results, cache identity, baking and saved-only replay. | Relocated baked bundle renders byte-identically to synthetic input; corruption/provenance/concurrent-save checks. Actual Fortran routine remains outside the implemented subset. |
| Delivery | Native presentation and PNG/MP4 export use the same evaluation/rendering. | Native controls/resize/reload smoke; 12-frame fractional-rate MP4; complete 480-frame 720p shot; exact direct/shuffled GPU seeks. |

This completes the agreed initial acceptance scene on the tested software
adapter. It does not establish all capabilities proposed in the design, native
Wayland or hybrid-GPU behavior, RTX/AMD performance, or integration with the
author's actual Fortran routine.

## Build and CPU/authoring checks

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm run typecheck
npm test
```

Final checks passed: 24 Rust library tests, two delivery integration tests, five
evaluation contracts, six host integration tests, one compile-fail immutability
doctest, and all 19 Node cases. The separate GPU integration test is ignored in
ordinary `cargo test`; a default pass alone establishes no rendered behavior.

Contracts include rational rates/exclusive endpoints, track boundaries, malformed
fields/references/text/scene properties, complete resolved state on direct and
shuffled seeks, immutable source/plan identities, numerical cache reuse, saved
payload and provenance revalidation, concurrent publication, encoder failure
cleanup, model transforms, and authored/resized stroke-width limits. Host client
tests distinguish recoverable scene errors from terminal protocol/process errors.
The real watch test publishes revision 1, retains its exact file through JS
construction and Rust validation errors, then publishes revision 2 in the same
host and exits cleanly on SIGINT.

In the restricted tool environment, Rust used the pre-fetched cache with
`CARGO_HOME=/tmp/lemma-cargo` and `--locked --offline`. Node subprocess tests
required approved sandbox escalation: the restricted run could otherwise report
only test-file wrappers without executing their internal cases. The recorded
19-case result is the actual subprocess-enabled run, with zero skipped cases.
Those temporary cache paths are not normal user setup requirements.

## Rendered GPU contracts

```sh
LEMMA_GPU_ADAPTER='NVIDIA' \
  cargo test --locked --test gpu_contracts -- --ignored --nocapture
# Deliberate software correctness run, when Mesa lavapipe is installed:
LEMMA_GPU_ALLOW_SOFTWARE=1 scripts/software-vulkan \
  cargo test --locked --test gpu_contracts -- --ignored --nocapture
```

An unavailable named adapter fails. The test refuses software unless explicitly
allowed. The full suite initializes one renderer and checks:

| Contract | Actual assertion |
| --- | --- |
| sRGB/capture layout | Every pixel of a 127×65 clear matches `[3, 21, 110, 255]` within one byte; padded rows are stripped. |
| Text alpha/linear composition | 962 half-opacity full-coverage glyph pixels match linear-light source-over within two bytes; alpha remains 255; zero opacity exactly matches blank. |
| Opaque depth/annotation | Behind-surface markers and `hide` labels exactly match the field-only image; front markers and `always` labels visibly change it. |
| Spatial strokes | Declared pixel width, zero reveal, opaque depth, and covered pixels at all four edges where the centerline lies outside the viewport. |
| Extreme finite path coordinates | Quarter/three-quarter reveals near `1e20` have distinct correct endpoints; visible sub-microunit near-camera paths survive clipping. |
| World text | Behind-surface text is occluded, front text is visible, and zero alpha leaves blank output. |
| Legend | Shifted/unshifted palette pixels match the piecewise-linear light-space gradient within quantization tolerance. |
| Animated model | Translation/rotation/uniform scale move surface and attachments together; a known rotated normal produces expected lighting within two bytes; marker/trace/plane/label depth remains correct. |
| Numerical upload reuse | Model/camera/palette edits change pixels with zero additional field uploads or bytes. |
| Fractional viewport | All nine covered centers survive a `[0.3, 0.3, 2.4, 2.4]` viewport in 3×3 output. |
| Retained GPU frames | Two simultaneously held outputs retain distinct results across later submissions and match independent rerenders exactly. |
| Direct/shuffled seeks | Reference frames 0, 120, 240, 479 at 320×180 remain byte-identical across six shuffled/repeated requests, with no additional field upload. |

Exact byte comparisons apply within this adapter environment. Cross-adapter
image identity is not asserted. Color tolerances cover quantization rather than
arbitrary image similarity.

Actual adapter:

```text
name:        llvmpipe (LLVM 21.1.8, 256 bits)
backend:     Vulkan
device_type: Cpu
driver:      llvmpipe
driver_info: Mesa 26.1.8 (LLVM 21.1.8)
software:    true
MSAA 4x:     supported
```

The final suite submitted 62 frames, with 60 in its timing aggregate (the two
held-frame submissions/readbacks are outside that aggregate). One run measured:

| Phase | Elapsed time |
| --- | ---: |
| Adapter/device/font/pipeline initialization | 61.869 ms |
| Reference compilation | 0.635 ms |
| Ten explicit reference evaluations | 0.267 ms total |
| CPU preparation/upload, 60 frames | 30.425 ms total |
| CPU command recording/submission, 60 frames | 3.407 ms total |
| Capture including GPU waits/mapping/readback, 60 frames | 805.405 ms total |

Five field uploads (4,295,820 bytes) across independent fixtures reflect deliberate
cache eviction between fixtures. Presentation-edit/seek tests separately require
no re-upload while their field is active. This is a correctness run, not a
hardware-throughput benchmark. GPU draw/upload duration is not isolated by CPU
timers. The log is `out/gpu-contracts-latest.log`.

The restricted shell's Vulkan loader was provided explicitly:

```sh
CARGO_HOME=/tmp/lemma-cargo \
LEMMA_GPU_ALLOW_SOFTWARE=1 \
LD_LIBRARY_PATH=/nix/store/vjdqsn0bz2llabfqa746zfgk4kmswba5-vulkan-loader-1.4.357.0/lib:/run/opengl-driver/lib \
scripts/software-vulkan \
  cargo test --locked --offline --test gpu_contracts -- --ignored --nocapture
```

That Nix store path is evidence for this machine, not a portable requirement.
Prefer `nix develop` and the runtime script in an ordinary development shell.
The restricted sandbox could not access the Nix daemon; no host configuration or
driver changes were made.

## Native preview

The checked-in diagnostic [check-preview.py](../scripts/check-preview.py) requires
Python 3, Xvfb, libX11, and ImageMagick. Run with the runtime libraries available:

```sh
scripts/software-vulkan python3 scripts/check-preview.py
```

Pass `--xvfb PATH`, `--x11-library PATH`, or `--image-tool PATH` if those optional
test dependencies are not on their usual paths. The test selects `llvmpipe`
explicitly by default (`--adapter` can override the name); it creates and closes
its own X11 server. An existing desktop is not controlled. The tool sandbox
required approved escalation to create the display socket.

The actual smoke test passed window creation, exact 240→241→240 stepping,
Home=0/End=479, play/pause and loop advancement, invalid reload retaining the
current frame, successful reload recovery, resize from 640×360 to 800×450,
screenshot capture, and clean Escape exit. The resized screenshot was inspected:
`out/preview-check/preview.png`. Logs and JSON report are in that same directory.
This verifies X11/software presentation; real Wayland/hybrid hardware presentation
remains untested.

## Saved replay and delivery

```sh
cargo run --locked -- bake-data --output out/baked/scene.json
cargo run --locked -- render-frame --scene out/baked/scene.json --require-saved \
  --frame 240 --size 640x360 --output out/replayed.png
cargo run --locked -- export --start 0 --end 12 --fps 30000/1001 \
  --size 320x180 --output out/verification-short.mp4
cargo run --locked -- export --size 1280x720 --output out/acceptance.mp4
ffprobe -v error -count_frames -show_streams out/acceptance.mp4
```

Add an explicit adapter, or `--allow-software` inside `scripts/software-vulkan`,
as appropriate. Add `--overwrite` for intentional reruns. FFmpeg must provide
`libx264`; use `--ffmpeg PATH` when it is not on PATH.

Measured evidence:

- The baked bundle was copied to a different directory; its relative saved
  references still resolved under `--require-saved`. Frame 240 at 640×360 was
  byte-identical to the synthetic PNG, SHA-256
  `f16cb512cdae33b5a7d8f74e2283416578245cf0adab4835e625529b031b56e0`.
- Saved-only PNG range `[240,243)` produced exactly three correctly named
  127×65 frames, exercising odd-width capture and half-open endpoints.
- FFprobe confirmed the short MP4 has exactly 12 H.264 frames at `30000/1001`,
  320×180, duration `0.400400` seconds.
- FFprobe confirmed the full shot has exactly 480 H.264 frames at `60/1`,
  1280×720, duration `8.000000` seconds. Both carry TV range, BT.709 matrix and
  primaries, and `iec61966-2-1` transfer metadata. Output uses yuv420p.
- The complete shot uploads one field (859,164 GPU buffer bytes) and eleven text
  assets. GPU resources, capture staging, and the encoder pipe are reused; no
  full image sequence is collected in memory. This is serialized bounded
  delivery, not an asynchronous encoding/throughput claim.

Outputs/reports: `out/acceptance.mp4`, `out/acceptance-export-report.json`,
`out/acceptance-{0000,0240,0479}.png`, `out/verification-short.mp4`,
`out/verification-short-report.json`, and `out/saved-replay-report.json`.
Export timing totals aggregate CPU preparation/upload and recording/submission
separately from evaluation, GPU wait/readback, and encoding/pipe waits. Renderer
snapshot `prepare_ms`/`submit_ms` describe its last frame; resource counters are
cumulative. These measurements do not establish RTX/AMD throughput or long-run
memory/stress behavior.
