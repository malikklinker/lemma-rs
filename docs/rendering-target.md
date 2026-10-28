# One initial rendering target

Status: agreed initial target, 2026-10-26. This specifies the acceptance scene and intended rendering vocabulary. A working focused backend now implements the initial scene; the broader vocabulary below remains design direction. See [the implementation design](implementation.md) for boundaries and criteria, and [verification](verification.md) for measured implementation evidence and limitations.

## Recommendation

Aim for one user-facing renderer and one initial acceptance scene. The earlier curve, mixed-plot, and field experiments become incremental steps within that scene. Keep the references as future capability and quality tests rather than requiring six separate reproductions up front.

"Render target" has two meanings here. The product target is the visual capability/scene we intend to deliver. GPU render targets are color/depth attachments and intermediate textures. One final image and one authoring model may require several internal passes and textures for depth, composition, masks, or glow. Neither one pipeline nor one physical texture is a requirement.

## Minimal drawing vocabulary

| Family | Public capability | References it serves |
| --- | --- | --- |
| Paths | 2D/3D curves, axes, arrows, reveal, per-sample colors, explicit stroke width | Curves in 2/4; trails in 1; plots and annotations throughout |
| Meshes and instances | Indexed geometry, per-instance transforms and numerical attributes, basic controlled shading, color maps and textures | Rigid figures in 1; planes/fields/surfaces in 6; markers in 2/4; the geometry basis for point sprites in 5 |
| 2D graphics layers | Groups, layout, shapes, text, pre-typeset math, images, opacity, and explicit placement | Equations/plots/legends in 1/2/3; palette panels in 6; screen overlays and world-space planes |

This is a conceptual vocabulary, not a promise that these all use the same shader or storage representation. Path, text, image, and mesh drawing can have different implementations while sharing frame evaluation, IDs, resources, and composition semantics. Higher-level components such as plots, legends, and field views build on these families.

Screen-space layers use layout coordinates. A world-space plane embeds 2D content in a 3D view. Projected annotations attach screen-sized content to world positions and declare an occlusion policy. A separate crowd or scientific plotting engine is not implied.

## Initial acceptance scene

A numerical field starts as a face-on flat map. The camera tilts while a height scale lifts the same sampled grid into a surface. A curve crosses it, colored markers identify sample locations, one readable label follows a marker, and an equation/legend stays fixed on screen.

Use a small deterministic synthetic field for the rendering experiment. The plane and surface share topology and sample coordinates; a height-scale parameter changes their representation. Starting with a perspective camera looking face-on avoids requiring an animated switch between camera projection types.

The same source data supplies field values, the trace, and marker attributes. Changing the camera or palette reuses those values when they suffice. A later data-provider integration can replace synthetic inputs with callable or baked outputs. Any height interpretation of fractal output is an explicit visual mapping, not a claim about its physical meaning.

### Required in the first scene

- One world view, camera, model transforms, and compatible color/depth handling.
- A sampled grid shown as a plane and displaced surface, with a palette-mapped scalar channel.
- One high-quality spatial curve and a small group of mesh instances/markers with per-instance attributes.
- Screen text/math and a legend, plus one projected annotation with explicit occlusion behavior.
- One timeline driving camera, transforms, reveal, and presentation parameters at an explicit sample time.
- A frame rendered directly at a requested time, with sequential and shuffled requests producing equivalent scene state.
- A normal GPU composition path that avoids CPU readback between 2D and 3D drawing.

### Follow-on appearance work

Add selective glow only after curve/text quality and color behavior pass. Blur and group effects need deliberate layer boundaries. Pre-typeset math assets are sufficient initially; automatic typesetting and symbol morphing remain separate work.

## What this proves and what follows

This scene exercises the shared foundation of references 2, 4, and 6. Its text and world-plane path establish part of reference 3. Its instance path establishes part of reference 1. It does not validate dense crowds, imported people assets, many overlapping transparent particles, general camera focus, volumetric rendering, or every motion graphics effect.

Extend it with focused tests when needed:

1. Replace marker geometry with a rigid figure and use a numerical force channel for color; scale population independently of solver work.
2. Add soft point sprites and test blending/overdraw for reference 5. Their geometry can reuse instancing, but their transparency and performance require separate evaluation.
3. Add spatial equation planes and focus styling for reference 3.

These extensions should reuse the scene, data, and timing model. They need not all be present to select an initial stack or deliver the first Fortran fractal video.

## Implementation within the one target

1. Render the curve and markers to establish projection, stroke quality, and exact-time sampling.
2. Add the sampled plane/grid, displacement, and shared depth.
3. Add 2D equation/legend content, a projected label, and a world-space graphics plane diagnostic.
4. Animate the complete shot and verify direct seeking, color/alpha, image quality, resource sharing, and native preview/headless frame output.
5. Connect the source adapter independently and test palette/camera changes without unnecessary numerical evaluation.

The public renderer should present a composition and requested sample time to its caller. Backend-specific modules translate resolved content into passes. Choosing Vello, Skia, a focused 3D module, or an existing 3D renderer remains an implementation decision evaluated against this scene.
