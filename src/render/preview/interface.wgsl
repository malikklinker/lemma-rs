@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

struct SceneOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vs_scene(@builtin(vertex_index) index: u32) -> SceneOut {
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: SceneOut;
    out.position = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment fn fs_scene(in: SceneOut) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv);
}

struct UiOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: vec4<f32>,
}
@vertex fn vs_ui(@location(0) position: vec4<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) shape: vec4<f32>) -> UiOut {
    var out: UiOut;
    out.position = position;
    out.uv = uv;
    out.color = color;
    out.shape = shape;
    return out;
}
@fragment fn fs_ui(in: UiOut) -> @location(0) vec4<f32> {
    var coverage = textureSample(image, image_sampler, in.uv).r;
    if in.shape.w > 0.5 {
        let radius = min(in.shape.z, min(in.shape.x, in.shape.y) * 0.5);
        let p = abs((in.uv - 0.5) * in.shape.xy) - (in.shape.xy * 0.5 - radius);
        let distance = length(max(p, vec2(0.0))) + min(max(p.x, p.y), 0.0) - radius;
        coverage *= 1.0 - smoothstep(-0.5, 0.5, distance);
    }
    let alpha = coverage * in.color.a;
    return vec4(in.color.rgb * alpha, alpha);
}
