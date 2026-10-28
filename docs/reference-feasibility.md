# Feasibility of the six visual references

Status: initial visual analysis and proposed implementation order, 2026-10-26. The originals are preserved in [references/](references/). No rendering implementation or benchmark is included.

The author supplied these six stills as desired visual references, followed by a [dense crowd example](references/reference-01-crowd.png) clarifying reference 1. The author confirms that the people have fixed rigid poses with no rigging, and color represents the force on each person. Other implementation techniques remain hypotheses; motion, camera freedom, sampling quality, and exact compositing requirements need to be checked against short clips or our own animated prototypes.

## Recommendation

Consolidate the first experiments into [one initial rendering target](rendering-target.md): a sampled field becomes a surface, with a spatial curve, colored markers, a projected label, and fixed 2D math/legend content. Reference 4's curve remains the first implementation step; references 2 and 6 guide the complete composition and numerical-field path. The detailed sequence below is a future expansion list, not six mandatory initial deliverables.

Reference 3 should guide text quality and composition throughout, with its focus effect added separately. Reference 1 establishes generic rigid mesh instances with per-instance numerical attributes as a useful extension; validate a small example before attempting its full crowd scale. Defer the dense transparency workload from reference 5 until the foundation works.

All six are feasible target classes. Relative difficulty below assumes cameras, asset loading, frame rendering, and the basic scene/timeline model exist. It is not an estimate of time to build the framework from an empty repository.

## Reference breakdown

| Reference | Visible composition | Plausible implementation | Relative work / main uncertainty |
| --- | --- | --- | --- |
| [1: people and trails](references/reference-01.png), [crowd clarification](references/reference-01-crowd.png) | Rigid unrigged figures, per-person force colors, ground markers/trails, environment geometry, and equations | Reused mesh geometry with instance transforms and scalar/vector attributes; a shared palette material, depth, basic shading, and optional glow | Moderate for the core capability; crowd size, mesh detail, transparency, focus, and simulation cost are separate scaling/polish concerns |
| [2: wave and plot layout](references/reference-02.png) | A complex-valued spatial curve, axes, markers, a density view, six small plots, and math labels | Bulk sampled curves and points in a 3D view, plus reusable 2D plot/layout components driven by the same source | Moderate; an excellent early test of line quality, camera projection, data mapping, and composition |
| [3: equations in depth](references/reference-03.png) | Sharp foreground math with dimmer, tilted, blurred background equations | Typeset equation assets on transformed planes; per-layer opacity/blur initially, optional depth-of-field later | Low-to-moderate for a designed layered scene; sharp animated text and general focus/overlap raise the scope |
| [4: glowing spatial curve](references/reference-04.png) | A colored curve wrapping around an axis with bright sample markers | A sampled parametric polyline with per-sample color, instanced markers, controlled stroke width, and glow | Low-to-moderate relative to the other scenes; high-quality strokes and temporal stability remain real engineering work |
| [5: dense cloud and highlighted slice](references/reference-05.png) | A spatial distribution of many soft points, a highlighted plane/slice, axes, and an overlaid curve | Instanced soft point sprites with explicit blending, a plane, and curve/label passes; evaluate a volume approach only if motion/semantics demand it | Highest performance/quality risk of these references; count, projected point size, overdraw, transparency, and data motion determine cost |
| [6: colored field and palette planes](references/reference-06.png) | A glowing field/ribbon and three colored rectangular planes in perspective | Scalar data texture or sampled mesh with GPU color mapping; transformed planes for the palette examples; glow | Moderate; a good first test of numerical fields, palette reuse, deformation, sampling, and depth |

### 1: rigid instances colored by numerical data

The author confirms fixed rigid poses with no rigging. Treat each person as an instance of shared mesh geometry, with a transform and numerical attributes. Position/orientation may change without deforming the figure. The supplied crowd image makes per-person force coloring a concrete data-driven rendering target.

The generic primitive is an instanced mesh with stable IDs and bulk position, orientation, scale, and scalar/vector channels. A shader maps a chosen force measure to a palette. Keep the numerical force available independently of its color and expose normalization, units, and range. Changing the palette should reuse the simulation result.

The reusable framework features are mesh instances, world-space trajectories, marker rings, simple environment meshes, and a screen-space equation. The social/physical model remains in user code; collision/navigation or crowd dynamics do not become implicit renderer features. A trail samples its source over an explicit time interval rather than accumulating positions only when preview frames render.

This uses genuine 3D mesh rendering, depth, and shading. Those are bounded core capabilities that also support scientific glyphs, arrows, particles, and repeated geometric objects. Imported asset formats, richer materials, and post-processing can expand separately; the reference does not imply a skeletal animation subsystem.

Start with a small opaque crowd and per-instance force colors. Then vary population, mesh detail, and visible pixel coverage before adding the reference's softer appearance. Instancing reduces duplicated geometry/submission overhead but does not eliminate triangle, shading, blending, or solver cost. Keep a shared material with per-instance attributes instead of allocating one independent material or authoring component per person. An existing implementation example is [Bevy custom instancing](https://bevy.org/examples/shaders/custom-shader-instancing/).

### 2: a strong core acceptance target

The left-hand view can be represented by a mapping such as `(x, real(psi), imaginary(psi))` into world coordinates. The probability-density view and small multiples are other views of the same numerical samples. This is an interpretation for our implementation, not a reconstruction of the original source.

This tests a valuable contract: one source, several representations, one timeline. Thin curves, phase colors, sample markers, axis labels, and plot alignment are more important here than elaborate lighting. Camera sampling and geometry generation should not force the numerical solver to rerun when only the view changes and the existing samples suffice.

### 3: spatial typography is a dedicated capability

Many desired depth cues can come from 2D content placed on planes with perspective, opacity, and explicit blur. A designed scene can use per-layer focus styling. General camera depth-of-field is a separate post-processing feature and should not be implied by that approximation.

Treat mathematical typesetting as an early asset requirement. Initially accept pre-typeset SVG/path or high-resolution image assets, including term colors. Automatic typesetting, term identity, and semantic equation morphing are additional capabilities. Texture resolution, filtering, and silhouette quality must hold up as equations move closer to the camera or tilt.

### 4: the smallest useful rendering experiment

Use a sampled 3D curve, per-sample color or a palette coordinate, a reveal interval, several markers, an axis, and a camera. Animate both the curve/markers and camera with exact-time evaluation. Use the result to test stroke quality before building a full plotting API.

Choose stroke semantics explicitly: screen-pixel width for diagram curves, or world-radius tubes where geometric thickness matters. Triangle-based strokes or tubes offer control over width, joins, caps, gradients, and depth. They need near-plane and degenerate-segment handling. A basic line-list example is not sufficient evidence of production-quality plotting.

Glow should preserve a sharp, readable core. It is an effect applied to selected content; its cost and appearance should be controlled independently of ordinary captions and equations.

### 5: a cloud is not automatically a continuous volume

Instanced camera-facing sprites can provide a dense point-cloud appearance. A continuous participating volume, with absorption/emission integrated through its interior, is a different rendering/data contract. The still alone does not establish which is necessary.

Start a separate particle stress scene with stable IDs and deterministic positions at sample time. Increase count and point size independently; transparent pixel coverage can dominate even when geometry is cheap. Avoid one JSX node or one host/native call per particle.

Choose blending according to intended meaning. Additive accumulation is convenient for luminous points, but changes density appearance and may saturate. Alpha compositing has ordering requirements; richer transparency methods have additional memory/work costs. Validate the chosen method with overlaps, bright slices, and camera rotation.

### 6: the bridge to the Fortran project

A scalar field should remain numerical input; its palette is presentation. Keep the field resident while animating palette parameters. A flat field can use a plane, while a deformed ribbon/height field needs explicit geometry and sample coordinates.

This supports a useful representation change: show a numerical result as a flat map, move that plane in space, and optionally displace geometry using a chosen channel. The fractal's numerical values are not automatically physical height; the visual mapping is an authoring decision.

Define whether colors are exact scientific encodings or artistic styling. Document interpolation, clipping, validity, and normalization. Bright glows and tone mapping can change perceived colors, so keep legends and unmodified views available when quantitative reading matters.

## Shared building blocks

These references suggest a focused initial scene vocabulary:

- Orthographic/perspective cameras, explicit view rectangles, transforms, and depth conventions.
- 2D groups, layout, paths, plots, text, and pre-typeset math assets.
- 3D polylines, markers, arrows/axes, and screen-sized or world-sized strokes.
- A 2D plane embedded in world space, plus projected labels with explicit occlusion policy.
- Sampled fields, textured planes, indexed surfaces, per-sample color, and palette lookup.
- Selected-content glow, blur, and an explicit color/alpha pipeline.
- Bulk instance data for later point clouds and reused models.
- A single source/timeline contract shared by every representation.

A large general character, lighting, or cinematic asset system is not needed to begin the selected experiments. Models, general depth-of-field, richer transparency, and true volumes remain explicit expansion points.

## Implementation sequence proposed

1. **Curve kernel from reference 4.** Prove camera projection, curve sampling, gradient, reveal, markers, antialiasing, and optional glow. Render exact sample times and test camera motion.
2. **Mixed scene from reference 2.** Add two views of the same samples, 2D small multiples, text/math assets, layout, and shared timing. Include one depth-aware annotation and a world-space plane to test mixing beyond a simple overlay.
3. **Field scene from reference 6.** Upload scalar data, apply/change a palette without rerunning the numerical source, and display a plane plus a deformed mesh. Connect this to the Fortran adapter when its real input/output signature is available.
4. **Typography scene from reference 3.** Reuse math assets on spatial planes, test large/oblique text, and add explicit focus styling. General depth-of-field remains separately scoped.
5. **Rigid instance scene from reference 1.** Render one reused mesh with bulk transforms, stable IDs, and force-based colors. Add trails and a small environment, then measure crowd scaling. Rigging is outside this confirmed reference's scope.
6. **Particle stress scene from reference 5.** Test quality and scaling with a deliberately chosen transparency model, then decide whether a volume path is justified.

The curve kernel is the first step within the consolidated acceptance scene. The remaining references guide extensions; they do not all need to ship before the fractal video. See [the single-target proposal](rendering-target.md) for the current initial scope.

## Library implications

The references support evaluating wgpu as the common GPU foundation, with Vello as a candidate for 2D drawing and a dedicated 3D path for curves, planes, meshes, and instances. Glow, depth, sampling, and composition are responsibilities of the overall stack, not guarantees obtained by selecting a 2D renderer.

Bevy remains a serious alternative when reusing existing rendering features saves more work than its adapter costs. Its official examples demonstrate HDR/emissive bloom, custom instancing, and depth-of-field. Those are reusable mechanisms, not proof that these reference looks or lemma's deterministic seeking work out of the box. Sources: [bloom](https://bevy.org/examples/3d-rendering/bloom-3d/), [instancing](https://bevy.org/examples/shaders/custom-shader-instancing/), [depth-of-field](https://docs.rs/bevy_post_process/latest/bevy_post_process/dof/index.html).

Choose between a focused 3D module and that existing infrastructure using the first mixed scene. Compare feature completeness, integration work, text/curve quality, and seek behavior alongside performance. No speed claim or backend commitment follows from still-image analysis.
