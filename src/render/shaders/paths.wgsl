struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(linear) local: vec2<f32>,
    @location(2) @interpolate(flat) shape: vec2<f32>,
}
@vertex fn vs_main(@location(0) position: vec4<f32>, @location(1) color: vec4<f32>, @location(2) local: vec2<f32>, @location(3) shape: vec2<f32>) -> Out {
    var out: Out;
    out.position = position;
    out.color = color;
    out.local = local;
    out.shape = shape;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let closest = vec2(clamp(in.local.x, 0.0, in.shape.x), 0.0);
    let distance = length(in.local - closest) - in.shape.y;
    let alpha = (1.0 - smoothstep(-0.65, 0.65, distance)) * in.color.a;
    if alpha < 0.01 { discard; }
    return vec4(in.color.rgb * alpha, alpha);
}
