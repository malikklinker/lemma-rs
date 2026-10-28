# Rendering library comparison

Status: researched recommendation, 2026-10-26. No renderer dependency has been adopted or benchmarked in this project.

## Recommendation

Evaluate a **shared wgpu foundation for mixed 2D and 3D rendering**, with a 2D drawing module and a focused scientific 3D path. Vello is the most direct 2D candidate for that shared-device experiment. Keep Skia in the comparison if its effects coverage justifies the work of proving efficient interop with the selected 3D renderer.

The previous Skia-first recommendation addressed 2D composition. The author has clarified that mixed 2D and 3D storytelling matters for future scientific projects, with Swaptube's 3D as a functional reference. Neither Skia nor Vello alone provides the required 3D scene renderer. See [mixed scene targets](mixed-scenes.md).

This is an integration and scope recommendation, not a performance ranking. The first Fortran fractal project stays 2D; a small mixed-scene proof should still precede a final stack decision.

The [six supplied visual references](reference-feasibility.md) now define that proof more concretely. The [single-target proposal](rendering-target.md) combines a spatial curve, sampled surface, numerical color mapping, and 2D graphics into one initial scene. Crisp mathematical assets and line quality are early criteria; glow follows the core quality checks. Model assets, general focus effects, and dense transparent clouds are later tests.

## Responsibilities

| Layer | Candidate | What it supplies |
| --- | --- | --- |
| Scene, animation, and source evaluation | lemma | Our semantics, scheduling, dependency graph, and caches |
| Box layout | Taffy | Sizing and placement using flex/grid/block algorithms |
| Text shaping and line layout | Skia text modules, or Parley | Glyph selection/positioning, line breaks, font fallback integration |
| 2D drawing and compositing | Skia or Vello | Paths, images, glyphs, blending, and supported effects |
| Scientific 3D | Focused wgpu module, or an integrated renderer such as Bevy | Cameras, meshes, points/curves, materials, and depth |
| GPU access and pass scheduling | Shared wgpu device/queue as leading candidate | Resource ownership, textures, depth attachments, submission, and cross-pass dependencies |
| Media encoding | FFmpeg initially | Encoded video/audio from rendered frames |

These libraries occupy different layers. None provides lemma's entire data, timeline, or JSX authoring model.

## Skia and Rust-Skia

Skia is a C++ graphics library. `skia-safe` supplies Rust wrappers for its drawing API, effects/shaders, text shaping/layout, and GPU backends. The `textlayout` feature exposes SkShaper/SkParagraph using HarfBuzz and ICU. GPU features must be enabled explicitly; the default configuration is CPU rendering. Sources: [Rust-Skia](https://github.com/rust-skia/rust-skia), [skia-safe features](https://github.com/rust-skia/rust-skia/blob/master/skia-safe/README.md).

For lemma, the attraction is an existing implementation of many features needed for motion graphics: composed image filters, blur, shadows, displacement, blending, and runtime shader effects. Sources: [image filters](https://api.skia.org/classSkImageFilters.html), [SkSL runtime effects](https://docs.skia.org/docs/user/sksl/).

The numerical field can remain separate from its appearance. Skia's raw image shaders bypass ordinary color conversion, which is useful when an image holds data rather than colors. A runtime effect can apply the palette. The spike must verify suitable float image formats, filtering, and precision on the chosen backend; raw shader support alone does not guarantee every numeric texture format. Source: [raw image shaders](https://docs.skia.org/docs/user/sksl/#raw-image-shaders).

Costs include a substantial C++ dependency, platform/backend setup, and native build/toolchain constraints. Rust-Skia downloads prebuilt binaries for supported configurations and otherwise builds Skia. Owning custom compute and sharing GPU resources can require backend-specific interop; do not assume a wgpu texture can be handed directly to Skia. Sources: [build instructions](https://github.com/rust-skia/rust-skia), [Skia Vulkan resource interop](https://github.com/google/skia/blob/main/site/docs/user/special/vulkan.md).

For a Skia-based path, evaluate a supported GPU backend through its established integration and verify texture ownership/synchronization with the chosen 3D backend. Native backend resource interop is possible; an automatic portable bridge from wgpu is not established here. The presence of Graphite support is not a reason to adopt it before checking required wrappers and features.

## Vello

Vello is a family of Rust 2D renderers. The current repository distinguishes:

- `vello_cpu`: CPU/SIMD/multithreaded rendering, described as its most mature option.
- `vello_gpu`: CPU path preprocessing followed by GPU rasterization/compositing, with wgpu and WebGL2 backends.
- `vello`: the original compute-centric renderer, now housed under `research/` and described as experimental.

The current `vello_gpu` name replaces `vello_hybrid`; repository naming and published crate availability can differ. Pin the exact selected release or revision rather than assuming these names are interchangeable. Source: [Vello GPU README](https://github.com/linebender/vello/blob/main/vello_gpu/README.md).

The Rust/wgpu path is attractive when custom GPU processing and GPU-produced textures are central. Its APIs include external-texture integration, but backend compatibility, resource lifetimes, and synchronization still need deliberate handling. Sources: [renderer selection](https://github.com/linebender/vello/blob/main/README.md), [architecture](https://github.com/linebender/vello/blob/main/ARCHITECTURE.md).

The current Vello GPU documentation lists mask layers, complex filter graphs, and certain non-isolated blend modes as unsupported, with some failures panicking. These are concrete gaps for our intended motion graphics API. Some could be supplied by lemma-owned offscreen passes, at additional implementation and performance cost. Do not expose unsupported features and let a long render panic. Source: [current Vello GPU limitations](https://github.com/linebender/vello/blob/main/vello_gpu/README.md#current-state).

Rendering glyph runs is only part of text support. A Vello-based stack can use Parley for rich text layout and its font/shaping stack, then pass positioned glyphs to the renderer. Source: [Parley](https://github.com/linebender/parley).

The mixed-scene requirement makes Vello a stronger candidate for the 2D module. Its wgpu renderer accepts a caller-supplied device, queue, command encoder, texture views, and texture bindings. This gives us a concrete shared-device integration point. Its optional depth attachment is documented for reducing 2D overdraw; it does not establish general 3D occlusion semantics. Source: [wgpu renderer API](https://github.com/linebender/vello/blob/main/vello_gpu/src/render/wgpu/mod.rs).

The missing motion graphics effects remain implementation work. The original compute renderer merits a separate experiment for vector-heavy scenes, with its experimental status made explicit.

## wgpu and Taffy

`wgpu` is a portable graphics and compute API. It handles GPU resources and work submission across native backends and web targets. It does not supply a complete text, path, or motion graphics system. Using it directly for every feature would put far more renderer work inside lemma. Source: [wgpu](https://wgpu.rs/).

It remains useful for native compute, scalar-field color mapping, particles, and custom passes when the selected compositor supports the needed integration. A Fortran CPU provider does not require us to use wgpu; its arrays can feed either compositor.

Taffy computes box layout using block, flexbox, and grid algorithms. It does not draw the boxes or implement a browser's entire CSS/DOM model. It is independent of the Skia/Vello choice, and text measurement must be integrated with it. Source: [Taffy](https://github.com/DioxusLabs/taffy).

## Choice for lemma

| Criterion | Skia path | Vello GPU path |
| --- | --- | --- |
| General motion graphics effects | Broad existing filter/shader facilities | Additional passes needed for documented gaps |
| Text | Integrated shaping/layout modules available | Assemble a text stack such as Parley plus glyph rendering |
| Rust-centric graphics stack | Rust API over C++ | Rust renderer with wgpu integration |
| Custom wgpu compute | Separate interop design | More natural shared-device integration, still requiring validation |
| Real 3D geometry and cameras | Needs a separate 3D renderer | Needs a separate 3D renderer |
| Shared 2D/3D GPU resources | Prove the chosen native backend bridge | Caller-owned wgpu device/encoder provides a direct integration point |
| Build/deployment | Prebuilt binaries where available; native tooling otherwise | Rust dependency build; verify selected release and GPU requirements |
| Fortran numerical arrays | Upload/map into supported image representation | Upload/map into supported image/texture representation |
| Performance | Measure our workloads | Measure our workloads |

With mixed scenes in scope, decide GPU resource ownership and 3D integration alongside the 2D drawing module. The leading experiment is wgpu plus Vello and a small scientific 3D pass. Compare that with reusing Bevy's renderer if the required 3D scope makes a dedicated implementation too costly. The source protocol and authoring scene model should keep these backend choices out of user numerical code.

Bevy offers a larger existing 2D/3D engine and render-to-texture support. Using it would require a lemma-owned adapter for exact-time scene updates, seeking, assets, and custom 2D pass integration. Its ECS and rendering schedules would remain implementation details. Sources: [Bevy](https://github.com/bevyengine/bevy), [render-to-texture example](https://bevy.org/examples/3d-rendering/render-to-texture/). Matching wgpu versions/devices is an integration requirement; creating independent devices does not yield freely shareable textures.

## Comparative spike

Use the same dimensions, fonts, sample times, alpha/color conventions, and supported visual operations:

1. Upload a numerical fractal field, map a palette, and animate the palette without calling the solver again.
2. Combine it with shaped text, a path reveal, translucent overlays, an animated mask, blur, and a shadow.
3. Exercise direct seeks, native preview, and headless frame output.
4. Add a small height field with an orbiting camera, a projected screen-space label, and a 2D graphic drawn on a plane. Check depth occlusion and preserve the same sampled timeline state on direct seeks.
5. Verify that 2D/3D exchange stays on the GPU in the normal path, then measure cold start, warm drawing, upload/readback, peak memory, and edit-to-preview latency separately from solver and encoder time.
6. Record implementation work needed for unsupported features and inspect output quality alongside timing.

A backend that lacks a required operation is not credited with a faster render of a simpler scene. First verify features and output, then compare throughput. Keep a narrow drawing/resource interface inside lemma; avoid an elaborate universal rendering abstraction before the examples reveal what it needs.
