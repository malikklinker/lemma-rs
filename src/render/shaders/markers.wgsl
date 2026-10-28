struct Camera { view_projection: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@vertex fn vs_main(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) instance: vec4<f32>, @location(3) color: vec4<f32>) -> Out {
    var out: Out;
    out.position = camera.view_projection * vec4(instance.xyz + p * instance.w, 1.0);
    out.normal = n;
    out.color = color;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let lighting = 0.45 + 0.55 * max(dot(normalize(in.normal), normalize(vec3(-0.4, 0.8, 0.6))), 0.0);
    return vec4(in.color.rgb * lighting, in.color.a);
}
