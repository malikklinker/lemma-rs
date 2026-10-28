@group(0) @binding(0) var glyph: texture_2d<f32>;
@group(0) @binding(1) var glyph_sampler: sampler;
__DEPTH_DECL__
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) anchor: vec4<f32>,
}
@vertex fn vs_main(@location(0) position: vec4<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) anchor: vec4<f32>) -> Out {
    var out: Out;
    out.position = position; out.uv = uv; out.color = color; out.anchor = anchor;
    return out;
}
fn shade(in: Out) -> vec4<f32> {
    let alpha = textureSample(glyph, glyph_sampler, in.uv).r * in.color.a;
    return vec4(in.color.rgb * alpha, alpha);
}
@fragment fn fs_world(in: Out) -> @location(0) vec4<f32> {
    let result = shade(in);
    if result.a < 0.01 { discard; }
    return result;
}
@fragment fn fs_overlay(in: Out) -> @location(0) vec4<f32> {
    if in.anchor.w > 0.5 {
        let coord = vec2<i32>(in.anchor.xy);
        let bounds = vec2<i32>(textureDimensions(scene_depth));
        if any(coord < vec2(0)) || any(coord >= bounds) { discard; }
        __DEPTH_READ__
        if in.anchor.z > depth + 0.00015 { discard; }
    }
    return shade(in);
}
