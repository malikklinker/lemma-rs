# lemma-rs

A deterministic motion graphics renderer for numerical data, with a Rust core
and TSX authoring. The initial scene turns one sampled scalar map into a surface,
with a spatial curve, instanced markers, projected annotation, world text, and
fixed equation/legend graphics.

The engine is runnable. The broader [design proposal](docs/design.md) describes
future capabilities; the supported boundary is the version 1 schema in
[src/scene.rs](src/scene.rs) and [authoring/index.ts](authoring/index.ts). See
[verification evidence](docs/verification.md) and the
[development log](docs/development-log.md) for measured results and changes.

## Build and render

On NixOS, enter the development shell for the Vulkan loader, window libraries,
and FFmpeg. It does not install or change host drivers.

```sh
nix develop
cargo build --locked
npm ci
cargo run --locked -- adapters
cargo run --locked -- render-frame --scene reference --frame 240 \
  --size 1280x720 --output out/frame-0240.png
```

Hardware selection is explicit when requested: add `--adapter 'NVIDIA'` or an
actual adapter-name substring from `adapters`. A missing selection fails.
Software adapters require `--allow-software`; they are useful for correctness
checks and do not establish hardware performance. To deliberately use Mesa
lavapipe, when its ICD is installed:

```sh
scripts/software-vulkan cargo run --locked -- render-frame \
  --allow-software --frame 240 --output out/software-frame.png
```

Headless capture/export reports include the actual adapter/driver and separate CPU
preparation/submission, evaluation, initialization, capture/wait, and encoding
measurements. GPU execution and upload duration are not isolated GPU timestamps.
Native preview logs its adapter and initialization time.
Generated output lives under ignored `out/`.

## Author and preview

```sh
npm run scene -- examples/reference.tsx --output out/reference.json
cargo run --locked -- inspect --scene out/reference.json --frame 240
npm run preview -- examples/reference.tsx --paused --frame 240
```

TSX function components execute on load/reload. `tween` and keyframe tracks are
serializable descriptions evaluated by Rust at an explicit integer frame and
positive rational frame rate. Arbitrary JavaScript does not execute per frame in
Rust or on the GPU. Set `<Composition fps={{ numerator: 30000, denominator: 1001 }}>`
to author a fractional rate. Rust render/preview commands accept `--fps 30000/1001`
as a sample-rate override; the authoring CLI forwards that flag only to preview.

`npm run watch -- examples/reference.tsx` validates edits through a persistent
Rust host before publishing the JSON file. Build errors keep the last valid
file. Preview watches the published scene and retains its last valid plan after
reload failures. Numerical results are cached within each process; the
validation host and preview currently have separate caches.

Native preview controls: Space play/pause, Left/Right frame step, Home/End seek,
L loop, R reload, Escape close. Resizing evaluates the same composition at the
new output size. Presentation samples the rendered GPU texture without CPU
readback. Preview requires an accessible X11 or Wayland display.

See [authoring and protocol](docs/authoring.md) for the supported schema and
process boundary.

## Save data and deliver

```sh
cargo run --locked -- bake-data --scene reference --output out/baked/scene.json
cargo run --locked -- render-frame --scene out/baked/scene.json --require-saved \
  --frame 240 --output out/replayed.png
cargo run --locked -- render-range --scene reference --start 0 --end 12 \
  --size 640x360 --output out/frames
cargo run --locked -- export --scene reference --start 0 --end 120 \
  --size 640x360 --output out/reference.mp4
```

Ranges are half-open: `[start, end)`. Omit `--end` to use the exclusive scene
duration at the selected frame rate. PNG sequences and MP4 exports refuse to
replace existing output unless `--overwrite` is given. Single-frame capture
replaces its output path. MP4 requires FFmpeg with `libx264` and positive even
pixel dimensions. Frames stream through bounded staging/pipe memory; no full
sequence is retained in RAM. Encoding is currently serialized with rendering.

`bake-data` writes content-identified little-endian binary32 fields plus validated
metadata and a scene with relative saved-data paths. `--require-saved` rejects
synthetic computation. Camera, palette, model, and annotation edits reuse
unchanged numerical results. Saved payloads and provenance are revalidated on
reload. The built-in synthetic provider and saved fields work; the actual
Fortran routine and asynchronous foreign-worker protocol are not integrated.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm run typecheck
npm test
# Opt-in rendered contracts on the selected hardware adapter:
LEMMA_GPU_ADAPTER='NVIDIA' cargo test --locked --test gpu_contracts -- --ignored --nocapture
# Explicit software correctness run:
LEMMA_GPU_ALLOW_SOFTWARE=1 scripts/software-vulkan \
  cargo test --locked --test gpu_contracts -- --ignored --nocapture
```

Default Rust tests do not run the GPU suite. The restricted development shell's
measured results use Mesa llvmpipe; RTX/AMD throughput requires hardware access.

## Current scope

The agreed [acceptance scene](docs/rendering-target.md) is the implementation
target, with [module boundaries and milestones](docs/implementation.md). The
renderer supports sampled scalar surfaces, round pixel-width spatial paths,
opaque sphere instances, one camera/world model, single-line bundled-font text,
legends, projected labels, and text planes. Model transforms support animated
translation, XYZ Euler rotation in degrees, and positive uniform scale.
Stroke widths are limited to 32,768 pixels, including after output scaling.

General mesh/image import, vector graphics/layout, automatic math typesetting,
group hierarchies/effects, glow, nonuniform transforms, and foreign numerical
workers remain future work. Text rejects unsupported glyphs. Paths, markers,
and scalar palettes require opaque colors; text supports alpha. World paths and
text test opaque geometry depth but do not write it, and blended world content
is not sorted. Projected-label `hide` checks opaque field/marker depth on the
GPU; text planes and paths do not occlude labels. SDR output is opaque sRGB PNG
or H.264 with explicit color metadata.
