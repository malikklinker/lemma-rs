# Authoring and host protocol

The implemented authoring boundary is `SceneSpec` version 1. TSX is bundled with
esbuild, evaluated once in a disposable Node worker, validated by Rust, and
published as JSON only after success. No frame callbacks cross this boundary.
The construction worker has a timeout and is terminated after each load to avoid
retaining arbitrary module state across reloads.

## Supported components

`Composition` accepts width/height, positive rational fps, duration in seconds, opaque
background, and an optional shared world `model`. `Camera` selects one perspective
camera and optional canvas-pixel viewport. `Field` declares a synthetic or saved
scalar asset. `Surface`, `Trace`, and `Markers` reference that immutable asset by
ID. `Polyline` supplies explicit world positions. `Text`, `Legend`, `Label`, and
`GraphicsPlane` provide the demonstrated mixed graphics.

The optional `model` has animated `translation`, `rotation_degrees`, and positive
uniform `scale`. Rotation uses XYZ Euler angles. All world content shares it,
including field-relative anchors and world graphics; screen layout and pixel
stroke widths stay in pixels. Numerical assets do not contain visual transforms.
This is one model transform, not a group hierarchy or general mesh scene graph.

`Animated<T>` is either a constant or an ordered keyframe track with `linear`,
`smooth`, `cubic-out`, or `hold` easing. Tracks clamp at their first/last keyframe; duplicate
or unordered times are errors. Built-in `tween` constructs this data. See
[the complete example](../examples/reference.tsx) and
[TypeScript definitions](../authoring/index.ts).

Rust rejects unknown fields/operations and reports node IDs for invalid content.
Compiled plans expose read-only spec/field getters; changing a scene requires
compilation. Field-coordinate samples are prepared during compilation. Frame
requests independently resolve tracks, camera, placement, and draw state.
Field interpolation follows the same triangle diagonal as the rendered mesh.
Stroke widths must be in `(0, 32768]` pixels both as authored and after output
scaling; unsupported translucent path colors are rejected at compilation.

## Persistent JSON-lines host

Start `target/debug/lemma serve [GPU OPTIONS]`. Standard output contains only
protocol messages; diagnostics go to standard error. Each request is one JSON
object with integer `version: 1`, a string or nonnegative integer `id`, and `op`.
The host responds with the same ID and either `ok: true, result` or
`ok: false, error: {code, message}`. Unknown operations and fields are errors.

```json
{"version":1,"id":1,"op":"load-path","path":"out/reference.json"}
{"version":1,"id":2,"op":"inspect-sample","frame":240}
{"version":1,"id":3,"op":"render-sample","frame":240,"output":"out/host-frame.png"}
{"version":1,"id":4,"op":"info"}
{"version":1,"id":5,"op":"shutdown"}
```

Supported operations:

| Operation | Payload and behavior |
| --- | --- |
| `load-scene` | `scene`, optional `base_dir`, optional `policy` (`on-demand` or `require-saved`); validates a candidate and publishes a new revision only after success. |
| `load-path` | `path`, optional `policy`; resolves saved assets relative to the scene file. |
| `inspect-sample` | Integer `frame`, optional rational `fps`, optional `size`; returns resolved state without initializing a GPU. |
| `render-sample` | Same sample parameters plus `output`; lazily initializes the selected renderer, captures a PNG, and reports timings/resources. |
| `info` | Revision, cache/renderer statistics, supported operations, and actual adapter if initialized. |
| `shutdown` | Acknowledges and exits. EOF also ends the host. |

A failed load retains the previous valid plan and revision. A host process owns
one source cache and at most one initialized GPU renderer. Dense numerical data
is referenced through saved binary assets; frame requests do not resend arrays.
The host bounds input-line size. The Node client correlates concurrent requests,
validates response envelopes, and treats malformed output, timeouts, pipe errors,
or process failures as terminal. Scene validation errors are recoverable.
Shutdown has its own short timeout.

The CLI's `--scene reference` uses the Rust fixture;
`examples/reference.tsx` is the editable frontend fixture. A parity test compares
them with schema defaults expanded and float32 tolerance. Their separate forms
exercise the process boundary without requiring Node for a built-in render.
