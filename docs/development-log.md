# Development log

## 2026-10-28: PR preparation and boundary validation

Three subagents reviewed core evaluation, TSX authoring, and delivery with
separate file ownership. The integration pass preserved the existing staged and
unstaged implementation and design documents; no commit or push was requested.

Completed fixes:

- Marker evaluation now rejects non-finite projected bounds with the stable node
  ID and guidance to reduce radius, model scale, or camera magnification. Valid
  offscreen markers remain supported, including shuffled direct seeks.
- Scene construction validates JSON data before worker transfer/publication,
  preventing non-finite numbers, cycles, class instances, and other unsupported
  values from being silently changed by serialization. Diagnostics identify the
  owning node and property path. Shared JSON values remain valid.
- Export resolves its first requested sample before creating output or starting
  FFmpeg/GPU initialization. A sample-dependent camera failure now retains its
  actionable diagnostic and leaves output untouched. The resolved first packet
  is reused and evaluation timing includes this work.
- GPU frames carry a renderer ownership token. Capture rejects foreign frames
  before GPU submission and replaces staging memory when reused with another
  renderer. A two-renderer regression covers rejection, recovery, equal-sized
  transfers, repeated capture, and switching back to the original renderer.

Integration evidence: `cargo fmt --all`, Clippy across all targets with warnings
denied, the full CPU Rust suite, TypeScript checking, and the subprocess-enabled
Node suite. The opt-in GPU suite passed on Vulkan llvmpipe, Mesa 26.1.8 / LLVM
21.1.8 (software CPU device, 4x MSAA), including the two-renderer capture test.
No hardware-throughput claim follows from these checks.

Rendered and visually inspected `out/pr-review-0240.png` at 640x360. A fresh
`out/pr-review-short.mp4` export covers `[0,12)` at 320x180; FFprobe confirms
H.264, exactly 12 frames, `30000/1001` fps, and 0.400400 seconds. Export reused
one field upload and eleven text assets. Commands follow the existing README
workflow with explicit `--allow-software` and `scripts/software-vulkan`; this
restricted shell supplied the Vulkan loader and FFmpeg paths explicitly.

These changes prepare the existing acceptance-scene implementation for review.
The repository still contains its pre-existing broad uncommitted implementation;
the integration pass does not establish a clean PR diff against a remote branch.

## 2026-10-26: acceptance-scene completion pass

Requested work window: up to five hours, beginning 10:09 a.m. America/Los_Angeles.
The working tree already contained the initial engine implementation and design
documents. All existing staged files and user changes are being preserved; no
commit or push is part of this pass.

### Initial review

Three independent reviews covered the core, GPU renderer, and authoring/host
loop before implementation changes. The existing code is a small, working
vertical slice, not an empty framework. Removing modules or replacing the
architecture would not address its main problems.

Concrete repeated work and correctness findings:

- Evaluation sampled unchanged field coordinates on every requested frame and
  evaluated the same height track once per point. Public mutable compiled-plan
  inputs also permitted bypassing compilation checks.
- Field attachments used bilinear interpolation, while GPU cells use two flat
  triangles. Coarse saved fields could put annotations/markers/traces below the
  visible surface.
- Legend gradients rebuilt 64 strips per frame despite using only five palette
  stops. Stroke clipping ignored width at viewport edges. Translucent path joins
  composited overlapping segment caps more than once.
- The authoring host client did not validate response envelopes or make stream
  failures terminal; its shutdown could wait for the normal long request timeout.
- Rust and TSX reference fixtures had drifted by one marker color component.
- Preview and export independently multiplied seconds by a rounded frame rate
  to find the exclusive endpoint; this can add an extra fractional-rate frame.
- The README and design status text described a design-only repository despite
  existing runnable rendering, authoring, saved-data, preview, and export code.

Baseline verification passed: 14 Rust unit tests, six host integration tests,
Clippy with warnings denied, TypeScript checking, and authoring tests. The GPU
integration suite is opt-in and was not counted as a default-test pass. A
1280×720 frame at sample 240 was rendered and visually inspected before changes
(`out/review-before-0240.png`). Its adapter was Mesa llvmpipe 26.1.8 / LLVM
21.1.8, Vulkan, software CPU device, with 4× MSAA. Hardware performance and native
window presentation remain unverified in this restricted shell.

Implementation and final verification entries below will record completed work,
commands, limitations, and inspected output rather than proposed capabilities.

### Implemented changes

| Area | Completed change and reason |
| --- | --- |
| `src/eval.rs` | Prepared field-coordinate samples at compilation and evaluated height once per node. Compiled plans expose read-only `spec()`/`fields()` access so postcompile mutation cannot invalidate cached samples or bypass checks. Expanded inspection to complete resolved styling, placements, legends, viewport, and model state. |
| `src/data.rs` | Matched attachment interpolation to the GPU grid's two triangles; removed the unused duplicate loading route. Enforced immutable scalar fields with public read-only getters, protecting values/range/content hash/provenance. Reused `AtomicOutput` instead of a duplicate PID-only temporary-name implementation, fixing reproduced concurrent-save corruption and cleaning failed publications. |
| `src/scene.rs`, `src/eval.rs` | Added an optional identity-default shared world model: animated translation, XYZ Euler degrees (`Rx * Ry * Rz`), and positive uniform scale. Applied it consistently to surfaces, paths, marker centers/radii, projected anchors, and world-text planes. Screen coordinates, label offsets/fonts, and pixel stroke widths remain in canvas pixels. |
| `src/render/{mod.rs,resources.rs,shaders/surface.wgsl}` | Applied model transforms to GPU surface positions and normals while reusing immutable field buffers. Normalized rotation separately from scale to avoid tiny/large-scale lighting overflow. Clarified last-frame CPU timings versus cumulative resource counters. |
| `src/render/paths.rs` | Expanded clipping side planes by stroke radius, preserving visible wide strokes whose centerlines are offscreen. Used f64 lengths/interpolation/clipping and normalized projected vertices to fix incorrect reveals at finite coordinates near `1e20`. Removed a fixed near-camera cutoff that discarded valid small geometry. |
| `src/scene.rs`, `src/eval.rs` | Rejected unsupported translucent path colors rather than double-compositing overlapping capsule joins. Added a public 32,768-pixel stroke-width ceiling, checked on load and after output scaling, with stable-node diagnostics. |
| `src/render/graphics.rs` | Replaced 64 uniform legend strips with the exact four/five shifted palette intervals; converted each palette stop once. This removes repeated work while improving stop-boundary correctness. |
| `src/time.rs`, `src/render/preview.rs`, `src/export.rs` | Shared one exclusive-duration frame-count rule using the same rational-sample conversion as evaluation. Avoided an extra endpoint from rounded fractional-fps multiplication. Preview/export no longer maintain separate endpoint calculations. |
| `src/export.rs` | Validated ranges/sizes before output creation/GPU initialization. Added range-wide CPU preparation/upload and recording/submission totals; included encoding setup in encoding/pipe timing. Verified failed encoders preserve previous output and remove partial files. Existing streaming bounded-memory delivery remains serialized. |
| `src/bin/lemma.rs`, `src/host.rs`, `src/render/preview.rs` | Migrated compiled-plan consumers to immutable getters without changing JSON protocol operations. |
| `authoring/host-client.mjs` | Validated version/ID/status/result/error response shapes and correlated reordered replies. Made malformed output, timeouts, process/pipe failures terminal for all pending/future requests. Preserved recoverable structured scene errors. Bounded idempotent shutdown separately from normal requests, with termination fallback. |
| `authoring/index.ts` | Added declarative model types/Composition support. Rejected unsupported component results and raw composition text with actionable errors instead of silently dropping content. |
| `examples/reference.tsx`, `src/reference.rs` | Animated the model along with camera/height/reveal. Fixed one drifted marker color and retained independent Rust/TSX fixtures guarded by schema-default/f32-tolerant parity. |
| `scripts/check-preview.py` | Added an opt-in reproducible native X11 smoke test using its own Xvfb display, explicit software adapter, keyboard controls, reload failure/recovery, resize, screenshot, and clean exit. Optional test tools are not engine dependencies. |
| Tests | Added evaluator/delivery contracts, concurrent-save/failure cleanup and immutability checks, expanded rendered GPU contracts, HostClient fixture tests, whole-reference parity, and real persistent watch recovery. Existing tests migrated to read-only APIs. |
| Documentation | Replaced the stale design-only README with runnable commands, scope, limits, source replay/export workflow, verification, and authoring/protocol guidance. Updated design-document status paragraphs while retaining their architectural proposals as proposals. |

### Compatibility and supported limits

- Scene JSON remains version 1; omitted model transforms default to identity.
  Saved field format/content identity is unchanged. Numerical inputs remain
  independent of model, camera, palette, and annotation edits.
- Rust callers now use read-only `CompiledPlan`/`ScalarField` getters and recompile
  scene edits. This deliberately closes previously mutable validation/identity
  boundaries; no stable external Rust API had been declared.
- Attachment sampling intentionally changes on nonplanar grid cells to match
  rendered triangles. Smooth fields differ slightly; coarse fields now attach
  correctly. This does not rewrite numerical payloads.
- Translucent paths and widths above 32,768 pixels now fail clearly. Opaque
  capsule paths remain the implemented stroke model. General transparent sorting,
  arbitrary meshes/images, nonuniform transforms/groups, automatic typesetting,
  Vello integration/effects, and foreign providers remain future capabilities.

### Verification and outputs

The full commands and precise assertions are in [verification.md](verification.md).
Current evidence includes the complete software-GPU suite (62 submitted frames),
all Rust/Node checks, and inspected saved images. The actual adapter remains
Vulkan llvmpipe, Mesa 26.1.8 / LLVM 21.1.8, software CPU, 4× MSAA. No RTX/AMD
throughput conclusion is drawn.

Native X11 smoke passed exact frame stepping, endpoint seeking, looping playback,
pause, failed reload retaining the active scene, recovery, resize, screenshot,
and clean exit. Its owned Xvfb display required approved sandbox escalation;
Wayland and hybrid hardware presentation are untested. Node's real 19-case
subprocess suite also required approved escalation. The restricted default
sandbox's file-only Node test output was not treated as sufficient evidence.

A baked scene was copied to another directory and replayed with computation
forbidden; its frame matched the synthetic source PNG exactly. A saved-only
odd-width range emitted exactly frames 240, 241, 242. FFprobe verified 12 frames at
30000/1001 over 0.400400 s and the complete 1280×720 shot at 60 fps, 480 frames,
8.000000 s, with explicit H.264 color metadata. The complete export reuses one
field upload and eleven text assets. Decoded beginning/middle/end frames were
saved for visual inspection.

Reviewable artifacts are under ignored `out/`: `acceptance.mp4`, explicit-sample
PNGs, decoded video PNGs, native preview screenshot/report, GPU contract log,
relocated baked data, and JSON timing/replay reports. No source dependencies were
added, no host configuration/driver changes were made, and no commit or push was
performed.
