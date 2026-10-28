struct Uniforms {
    view_projection: mat4x4<f32>,
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
    params: vec4<f32>,
    palette: array<vec4<f32>, 5>,
    style: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) value: f32,
    @location(2) grid: vec2<f32>,
}
@vertex fn vs_main(@location(0) raw: vec3<f32>, @location(1) gradient: vec2<f32>, @location(2) grid: vec2<f32>) -> Out {
    var out: Out;
    out.position = u.view_projection * u.model * vec4(raw.x, raw.y * u.params.x, raw.z, 1.0);
    let slope = clamp(gradient * u.params.x, vec2(-1e18), vec2(1e18));
    out.normal = (u.normal * vec4(normalize(vec3(-slope.x, 1.0, -slope.y)), 0.0)).xyz;
    out.value = raw.y;
    out.grid = grid;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let t = clamp((in.value - u.params.y) / (u.params.z - u.params.y) + u.params.w, 0.0, 1.0) * 4.0;
    let i = min(u32(t), 3u);
    let base = mix(u.palette[i].rgb, u.palette[i + 1u].rgb, t - f32(i));
    let diffuse = abs(dot(normalize(in.normal), normalize(vec3(0.3, 0.9, 0.4))));
    let lighting = 0.69 + 0.31 * diffuse;
    let coord = in.grid / 8.0;
    let distance = abs(fract(coord - 0.5) - 0.5) / max(fwidth(coord), vec2(0.00001));
    let line = 1.0 - clamp(min(distance.x, distance.y), 0.0, 1.0);
    return vec4(base * lighting * (1.0 - u.style.x * line), 1.0);
}
