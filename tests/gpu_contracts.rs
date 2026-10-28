//! Opt-in rendered contracts. Run with `cargo test --test gpu_contracts -- --ignored
//! --nocapture`; software adapters additionally require LEMMA_GPU_ALLOW_SOFTWARE=1.
use anyhow::{Context, Result, ensure};
use lemma::{
    animation::tween,
    eval::{CompiledPlan, FramePacket, GraphicDraw, LegendDraw, MarkerDraw, PathDraw, Placement},
    render::{Renderer, capture::Capture, gpu::GpuOptions},
    scene::{
        Anchor, CameraSpec, NodeSpec, Occlusion, Palette, SceneSpec, TransformSpec, srgb_to_linear,
    },
};
use std::{path::Path, time::Instant};

#[derive(Default)]
struct Readbacks {
    capture: Capture,
    count: u64,
    wait_and_readback_ms: f64,
    prepare_ms: f64,
    record_and_submit_ms: f64,
}

impl Readbacks {
    fn render(&mut self, renderer: &mut Renderer, packet: &FramePacket) -> Result<Vec<u8>> {
        let frame = renderer.render(packet)?;
        self.prepare_ms += renderer.stats.prepare_ms;
        self.record_and_submit_ms += renderer.stats.submit_ms;
        let start = Instant::now();
        let pixels = self.capture.read_rgba8(renderer, &frame)?;
        self.wait_and_readback_ms += start.elapsed().as_secs_f64() * 1000.0;
        self.count += 1;
        ensure!(pixels.len() == (packet.size[0] * packet.size[1] * 4) as usize);
        Ok(pixels)
    }
}

fn empty_scene() -> SceneSpec {
    let mut scene = lemma::reference::scene();
    scene.size = [128, 128];
    scene.assets.clear();
    scene.nodes.clear();
    scene.background = [0.2, 0.35, 0.5, 1.0];
    scene.camera = CameraSpec {
        viewport: None,
        eye: [0.0, 4.0, 0.0].into(),
        target: [0.0, 0.0, 0.0].into(),
        up: [0.0, 0.0, -1.0],
        fov_y_degrees: 45.0,
        near: 0.1,
        far: 100.0,
    };
    scene
}

fn packet(scene: SceneSpec) -> Result<FramePacket> {
    let plan = CompiledPlan::compile(scene, Path::new("."))?;
    plan.evaluate(plan.spec().fps.sample(0), None)
}

fn changed_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

fn expect_color(actual: &[u8], expected: [u8; 4], tolerance: u8) {
    for (channel, (&actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "channel {channel}: actual {actual}, expected {expected}, tolerance {tolerance}"
        );
    }
}

fn linear_to_srgb_byte(linear: f32) -> u8 {
    let encoded = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn srgb_background(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let expected = [3, 21, 110, 255];
    let mut scene = empty_scene();
    // An odd width also protects padding removal during texture readback.
    scene.size = [127, 65];
    scene.background = expected.map(|c| f32::from(c) / 255.0);
    let pixels = readbacks.render(renderer, &packet(scene)?)?;
    for pixel in pixels.chunks_exact(4) {
        expect_color(pixel, expected, 1);
    }
    println!(
        "PASS sRGB clear: 127x65 pixels within one byte, including alpha and padded-row removal"
    );
    Ok(())
}

fn text_alpha(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let background = [0.2, 0.35, 0.5, 1.0];
    let color = [0.9, 0.6, 0.2, 1.0];
    let mut full = packet(empty_scene())?;
    full.graphics.push(GraphicDraw {
        id: "alpha-contract".into(),
        text: "MMMM".into(),
        font_size: 40.0,
        color,
        placement: Placement::Screen {
            position: [4.0, 25.0],
        },
    });
    let opaque = readbacks.render(renderer, &full)?;
    let mut half = full.clone();
    half.graphics[0].color[3] = 0.5;
    let translucent = readbacks.render(renderer, &half)?;
    let foreground = srgb_to_linear(color);
    let background = srgb_to_linear(background);
    let expected = [
        linear_to_srgb_byte((foreground[0] + background[0]) * 0.5),
        linear_to_srgb_byte((foreground[1] + background[1]) * 0.5),
        linear_to_srgb_byte((foreground[2] + background[2]) * 0.5),
        255,
    ];
    let opaque_color = color.map(|v| (v * 255.0).round() as u8);
    let mut interior = 0;
    for (opaque, half) in opaque.chunks_exact(4).zip(translucent.chunks_exact(4)) {
        assert_eq!(
            half[3], 255,
            "compositing over opaque background keeps alpha"
        );
        if opaque
            .iter()
            .zip(opaque_color)
            .all(|(&a, b)| a.abs_diff(b) <= 1)
        {
            expect_color(half, expected, 2);
            interior += 1;
        }
    }
    assert!(
        interior > 200,
        "opacity test needs substantial full-coverage glyph interiors"
    );
    half.graphics[0].color[3] = 0.0;
    let transparent = readbacks.render(renderer, &half)?;
    let blank = readbacks.render(renderer, &packet(empty_scene())?)?;
    assert_eq!(
        transparent, blank,
        "zero-alpha graphics must leave no color fringe"
    );
    println!(
        "PASS linear premultiplied text alpha: {interior} interior pixels within two bytes; zero opacity matches blank"
    );
    Ok(())
}

fn negative_glyph_bearings(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let font = fontdue::Font::from_bytes(
        include_bytes!("../assets/DejaVuSans.ttf") as &[u8],
        fontdue::FontSettings::default(),
    )
    .unwrap();
    for character in ['J', 'j'] {
        let (metrics, bitmap) = font.rasterize(character, 256.0);
        assert!(metrics.xmin < -1, "fixture must have a negative bearing");
        let mut scene = empty_scene();
        scene.size = [192, 352];
        scene.background = [0.0, 0.0, 0.0, 1.0];
        scene.nodes.push(NodeSpec::Text {
            id: "negative-bearing".into(),
            text: character.to_string(),
            position: [16.0, 16.0],
            font_size: 256.0,
            color: [1.0; 4],
            opacity: 1.0.into(),
        });
        let pixels = readbacks.render(renderer, &packet(scene)?)?;
        // White over black converts glyph coverage directly to sRGB. Compare
        // every covered pixel, independent of the glyph's placement/padding.
        let mut expected: Vec<_> = bitmap
            .into_iter()
            .filter(|&coverage| coverage > 0)
            .map(|coverage| linear_to_srgb_byte(f32::from(coverage) / 255.0))
            .collect();
        let mut actual: Vec<_> = pixels
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .filter(|&coverage| coverage > 2)
            .collect();
        assert_eq!(
            actual.len(),
            expected.len(),
            "{character}: clipped glyph coverage"
        );
        expected.sort_unstable();
        actual.sort_unstable();
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= 1,
                "{character}: changed glyph coverage"
            );
        }
    }
    println!("PASS negative glyph bearings: J/j preserve every rasterized coverage pixel");
    Ok(())
}

fn field_scene() -> SceneSpec {
    let reference = lemma::reference::scene();
    let mut scene = empty_scene();
    scene.assets = reference.assets;
    let mut surface = reference.nodes.into_iter().next().unwrap();
    if let NodeSpec::Surface { height, grid, .. } = &mut surface {
        *height = 0.0.into();
        *grid = 0.0;
    }
    scene.nodes.push(surface);
    scene
}

fn opaque_depth_and_labels(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let base = packet(field_scene())?;
    let field = readbacks.render(renderer, &base)?;
    let mut behind = base.clone();
    behind.markers.push(MarkerDraw {
        position: [0.0, -0.5, 0.0],
        radius: 0.25,
        color: [1.0, 0.0, 0.0, 1.0],
    });
    let hidden = readbacks.render(renderer, &behind)?;
    assert_eq!(field, hidden, "opaque field must hide a marker behind it");
    behind.markers[0].position[1] = 0.5;
    let front = readbacks.render(renderer, &behind)?;
    assert!(
        changed_pixels(&field, &front) > 200,
        "marker in front must be visible"
    );

    let mut label = base.clone();
    label.graphics.push(GraphicDraw {
        id: "depth-label".into(),
        text: "DEPTH".into(),
        font_size: 24.0,
        color: [1.0; 4],
        placement: Placement::Projected {
            anchor: [0.0, -0.5, 0.0],
            offset: [-40.0, -15.0],
            occlusion: Occlusion::Hide,
        },
    });
    assert_eq!(
        field,
        readbacks.render(renderer, &label)?,
        "Hide policy must hide the whole label at an occluded anchor"
    );
    if let Placement::Projected { occlusion, .. } = &mut label.graphics[0].placement {
        *occlusion = Occlusion::Always;
    }
    let always = readbacks.render(renderer, &label)?;
    assert!(
        changed_pixels(&field, &always) > 200,
        "Always policy must show an occluded label"
    );
    if let Placement::Projected {
        anchor, occlusion, ..
    } = &mut label.graphics[0].placement
    {
        anchor[1] = 0.5;
        *occlusion = Occlusion::Hide;
    }
    let visible = readbacks.render(renderer, &label)?;
    assert!(
        changed_pixels(&field, &visible) > 200,
        "Hide policy must show an unoccluded label"
    );
    println!("PASS opaque field/marker depth and projected-label Hide/Always policies");
    Ok(())
}

fn spatial_strokes(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut blank = packet(empty_scene())?;
    blank.view_projection = glam::Mat4::IDENTITY;
    let background = readbacks.render(renderer, &blank)?;
    let mut line = blank.clone();
    line.paths.push(PathDraw {
        id: "stroke-contract".into(),
        points: vec![[-0.75, 0.0, 0.5], [0.75, 0.0, 0.5]],
        width: 8.0,
        color: [1.0; 4],
        reveal: 1.0,
    });
    let pixels = readbacks.render(renderer, &line)?;
    let center_column: Vec<_> = (0..128)
        .filter(|&y| {
            let offset = (y * 128 + 64) * 4;
            pixels[offset..offset + 4] != background[offset..offset + 4]
        })
        .collect();
    assert!(
        (8..=12).contains(&center_column.len()),
        "an eight-pixel stroke must retain its pixel width, got {} covered rows",
        center_column.len()
    );
    for y in 62..66 {
        let offset = (y * 128 + 64) * 4;
        expect_color(&pixels[offset..offset + 4], [255; 4], 1);
    }
    line.paths[0].reveal = 0.0;
    assert_eq!(readbacks.render(renderer, &line)?, background);
    line.paths[0].reveal = 1.0;

    // Centerlines two pixels beyond each viewport edge still cover two pixels
    // inside it. Clipping an unexpanded centerline used to erase these strokes.
    for axis in 0..2 {
        for side in [-1.0, 1.0] {
            let mut points = [[0.0, 0.0, 0.5]; 2];
            for (i, point) in points.iter_mut().enumerate() {
                point[axis] = side * (1.0 + 4.0 / 128.0);
                point[1 - axis] = if i == 0 { -0.5 } else { 0.5 };
            }
            line.paths[0].points = points.to_vec();
            let pixels = readbacks.render(renderer, &line)?;
            let coordinate = if side < 0.0 { 0 } else { 127 };
            let (x, y) = if axis == 0 {
                (coordinate, 64)
            } else {
                (64, if side < 0.0 { 127 } else { 0 })
            };
            let offset = (y * 128 + x) * 4;
            expect_color(&pixels[offset..offset + 4], [255; 4], 2);
            assert!(changed_pixels(&background, &pixels) >= 100);
        }
    }

    let mut depth = packet(field_scene())?;
    let field = readbacks.render(renderer, &depth)?;
    depth.paths.push(PathDraw {
        id: "depth-stroke".into(),
        points: vec![[-1.0, -0.5, 0.0], [1.0, -0.5, 0.0]],
        width: 8.0,
        color: [1.0; 4],
        reveal: 1.0,
    });
    assert_eq!(
        field,
        readbacks.render(renderer, &depth)?,
        "opaque field must hide strokes behind it"
    );
    for point in &mut depth.paths[0].points {
        point[1] = 0.5;
    }
    assert!(changed_pixels(&field, &readbacks.render(renderer, &depth)?) > 200);
    println!(
        "PASS spatial stroke pixel width, zero reveal, all four viewport edges, and opaque depth"
    );
    Ok(())
}

fn world_graphics_depth(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut scene = packet(field_scene())?;
    let field = readbacks.render(renderer, &scene)?;
    scene.graphics.push(GraphicDraw {
        id: "world-depth-contract".into(),
        text: "DEPTH".into(),
        font_size: 40.0,
        color: [1.0; 4],
        placement: Placement::World {
            position: [-0.8, -0.5, -0.2],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, -1.0],
            width: 1.6,
        },
    });
    assert_eq!(
        field,
        readbacks.render(renderer, &scene)?,
        "opaque field must hide a text plane behind it"
    );
    if let Placement::World { position, .. } = &mut scene.graphics[0].placement {
        position[1] = 0.5;
    }
    assert!(
        changed_pixels(&field, &readbacks.render(renderer, &scene)?) > 100,
        "a text plane in front of the field must be visible"
    );
    scene.graphics[0].color[3] = 0.0;
    assert_eq!(field, readbacks.render(renderer, &scene)?);
    println!("PASS world text-plane opaque depth and zero-alpha discard");
    Ok(())
}

fn extreme_coordinate_strokes(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut scene = empty_scene();
    scene.camera = CameraSpec {
        viewport: None,
        eye: [0.0, 0.0, 0.0].into(),
        target: [0.0, 0.0, -1.0].into(),
        up: [0.0, 1.0, 0.0],
        fov_y_degrees: 120.0,
        near: 0.1,
        far: 1e22,
    };
    scene.nodes.push(NodeSpec::Polyline {
        id: "large-coordinate-stroke".into(),
        points: vec![[-1e20, 0.0, -1e20], [1e20, 0.0, -1e20]],
        width: 8.0,
        color: [1.0; 4],
        reveal: 0.25.into(),
    });
    let quarter = packet(scene.clone())?;
    let mut blank = quarter.clone();
    blank.paths.clear();
    let background = readbacks.render(renderer, &blank)?;
    let quarter_pixels = readbacks.render(renderer, &quarter)?;
    if let NodeSpec::Polyline { reveal, .. } = &mut scene.nodes[0] {
        *reveal = 0.75.into();
    }
    let three_quarters = packet(scene)?;
    let three_quarter_pixels = readbacks.render(renderer, &three_quarters)?;
    let center = (64 * 128 + 64) * 4;
    assert_eq!(
        quarter_pixels[center..center + 4],
        background[center..center + 4],
        "quarter reveal must stop before the midpoint even with 1e20 world coordinates"
    );
    expect_color(&three_quarter_pixels[center..center + 4], [255; 4], 1);
    assert!(changed_pixels(&quarter_pixels, &three_quarter_pixels) > 200);

    // The near plane may be closer than one microunit. A fixed W epsilon must
    // not erase a valid segment in front of that camera.
    let mut small = empty_scene();
    small.camera = CameraSpec {
        viewport: None,
        eye: [0.0, 0.0, 0.0].into(),
        target: [0.0, 0.0, -1.0].into(),
        up: [0.0, 1.0, 0.0],
        fov_y_degrees: 60.0,
        near: 1e-8,
        far: 1.0,
    };
    small.nodes.push(NodeSpec::Polyline {
        id: "small-coordinate-stroke".into(),
        points: vec![[-2e-8, 0.0, -1e-7], [2e-8, 0.0, -1e-7]],
        width: 4.0,
        color: [1.0; 4],
        reveal: 1.0.into(),
    });
    let small = readbacks.render(renderer, &packet(small)?)?;
    assert!(changed_pixels(&background, &small) > 100);
    println!(
        "PASS 1e20-coordinate stroke reveal distinguishes quarter/three-quarter endpoints; sub-microunit camera depths remain visible"
    );
    Ok(())
}

fn palette_legend(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut scene = packet(empty_scene())?;
    let palette = Palette::default();
    scene.legends.push(LegendDraw {
        position: [0.0, 32.0],
        size: [128.0, 32.0],
        palette: palette.clone(),
        shift: 0.0,
    });
    let colors = palette.stops.map(srgb_to_linear);
    for shift in [-0.31, 0.0, 0.31] {
        scene.legends[0].shift = shift;
        let pixels = readbacks.render(renderer, &scene)?;
        for x in 0..128 {
            let t = ((x as f32 + 0.5) / 128.0 + shift).clamp(0.0, 1.0) * 4.0;
            let i = (t as usize).min(3);
            let expected = std::array::from_fn(|channel| {
                if channel == 3 {
                    255
                } else {
                    linear_to_srgb_byte(
                        colors[i][channel]
                            + (colors[i + 1][channel] - colors[i][channel]) * (t - i as f32),
                    )
                }
            });
            let offset = (48 * 128 + x) * 4;
            expect_color(&pixels[offset..offset + 4], expected, 2);
        }
    }
    println!("PASS exact linear-light palette legend at unshifted and shifted stop boundaries");
    Ok(())
}

fn model_transforms(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut scene = field_scene();
    let surface_color = [0.72, 0.58, 0.35, 1.0];
    if let NodeSpec::Surface { palette, .. } = &mut scene.nodes[0] {
        palette.stops = [surface_color; 5];
    }
    scene.model = TransformSpec {
        translation: tween([0.0; 3], [0.4, 0.15, -0.2], 0.0, 1.0),
        rotation_degrees: tween([0.0; 3], [0.0, 0.0, 60.0], 0.0, 1.0),
        scale: tween(1.0, 0.65, 0.0, 1.0),
    };
    let plan = CompiledPlan::compile(scene.clone(), Path::new("."))?;
    let original = plan.evaluate(plan.spec().fps.sample(0), None)?;
    let transformed = plan.evaluate(plan.spec().fps.sample(60), None)?;
    assert!(std::sync::Arc::ptr_eq(
        &original.surfaces[0].field,
        &transformed.surfaces[0].field
    ));
    let original_pixels = readbacks.render(renderer, &original)?;
    let uploads = renderer.stats.field_uploads;
    let bytes = renderer.stats.field_upload_bytes;
    let transformed_pixels = readbacks.render(renderer, &transformed)?;
    assert!(changed_pixels(&original_pixels, &transformed_pixels) > 1000);

    // A flat surface has a known normal. Rotating the model must rotate its
    // lighting as well as its position, while uniform scale has no effect on it.
    let rotation = glam::Quat::from_rotation_z(60.0_f32.to_radians());
    let normal = rotation * glam::Vec3::Y;
    let light = glam::Vec3::new(0.3, 0.9, 0.4).normalize();
    let lighting = 0.69 + 0.31 * normal.dot(light).abs();
    let linear = srgb_to_linear(surface_color);
    let expected = [
        linear_to_srgb_byte(linear[0] * lighting),
        linear_to_srgb_byte(linear[1] * lighting),
        linear_to_srgb_byte(linear[2] * lighting),
        255,
    ];
    let offset = (64 * 128 + 64) * 4;
    expect_color(&transformed_pixels[offset..offset + 4], expected, 2);

    let attachments = [
        NodeSpec::Markers {
            id: "transformed-marker".into(),
            field: "field".into(),
            points: vec![[0.0, 0.0]],
            height: 0.0.into(),
            lift: -0.5,
            radius: 0.2,
            colors: vec![[1.0; 4]],
        },
        NodeSpec::Trace {
            id: "transformed-trace".into(),
            field: "field".into(),
            points: vec![[-1.0, 0.0], [1.0, 0.0]],
            height: 0.0.into(),
            lift: -0.5,
            width: 8.0,
            color: [1.0; 4],
            reveal: 1.0.into(),
        },
        NodeSpec::GraphicsPlane {
            id: "transformed-plane".into(),
            text: "DEPTH".into(),
            position: [-0.8, -0.5, -0.2].into(),
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, -1.0],
            width: 1.6,
            font_size: 40.0,
            color: [1.0; 4],
        },
        NodeSpec::Label {
            id: "transformed-label".into(),
            text: "DEPTH".into(),
            anchor: Anchor::World {
                position: [0.0, -0.5, 0.0].into(),
            },
            offset: [-30.0, -12.0],
            font_size: 24.0,
            color: [1.0; 4],
            occlusion: Occlusion::Hide,
        },
    ];
    for mut attachment in attachments {
        let evaluate = |attachment: NodeSpec| -> Result<FramePacket> {
            let mut attached = scene.clone();
            attached.nodes.push(attachment);
            let plan = CompiledPlan::compile(attached, Path::new("."))?;
            plan.evaluate(plan.spec().fps.sample(60), None)
        };
        let behind = evaluate(attachment.clone())?;
        assert_eq!(
            transformed_pixels,
            readbacks.render(renderer, &behind)?,
            "transformed surface must hide attached node {:?} behind its local plane",
            attachment.id()
        );
        match &mut attachment {
            NodeSpec::Markers { lift, .. } | NodeSpec::Trace { lift, .. } => *lift = 0.5,
            NodeSpec::GraphicsPlane { position, .. } => *position = [-0.8, 0.5, -0.2].into(),
            NodeSpec::Label { anchor, .. } => {
                *anchor = Anchor::World {
                    position: [0.0, 0.5, 0.0].into(),
                }
            }
            _ => unreachable!(),
        }
        let front = evaluate(attachment.clone())?;
        assert!(
            changed_pixels(&transformed_pixels, &readbacks.render(renderer, &front)?) > 20,
            "attached node {:?} in front of the transformed surface must remain visible",
            attachment.id()
        );
    }
    assert_eq!(original_pixels, readbacks.render(renderer, &original)?);
    assert_eq!(renderer.stats.field_uploads, uploads);
    assert_eq!(renderer.stats.field_upload_bytes, bytes);
    println!(
        "PASS animated model translation/rotation/scale, rotated surface lighting, transformed marker/trace/plane/label depth, and zero additional field uploads"
    );
    Ok(())
}

fn field_cache(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut base = packet(field_scene())?;
    let original = readbacks.render(renderer, &base)?;
    let uploads = renderer.stats.field_uploads;
    let bytes = renderer.stats.field_upload_bytes;
    let field = base.surfaces[0].field.clone();
    base.eye = glam::Vec3::new(0.4, 4.0, 0.4);
    let projection = glam::Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 100.0);
    base.view_projection =
        projection * glam::Mat4::look_at_rh(base.eye, glam::Vec3::ZERO, glam::Vec3::NEG_Z);
    let moved = readbacks.render(renderer, &base)?;
    assert!(
        changed_pixels(&original, &moved) > 100,
        "camera edit must change the image"
    );
    base.surfaces[0].palette.stops.reverse();
    base.surfaces[0].shift = 0.1;
    let recolored = readbacks.render(renderer, &base)?;
    assert!(
        changed_pixels(&moved, &recolored) > 100,
        "palette edit must change the image"
    );
    assert!(std::sync::Arc::ptr_eq(&field, &base.surfaces[0].field));
    assert_eq!(
        renderer.stats.field_uploads, uploads,
        "presentation edits must reuse the field upload"
    );
    assert_eq!(renderer.stats.field_upload_bytes, bytes);
    assert_eq!(renderer.stats.cached_fields, 1);
    println!(
        "PASS camera/palette edits change pixels with zero additional field uploads or field bytes"
    );
    Ok(())
}

fn fractional_viewport(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut scene = field_scene();
    scene.size = [10, 10];
    scene.camera.viewport = Some([1.0, 1.0, 8.0, 8.0]);
    let plan = CompiledPlan::compile(scene, Path::new("."))?;
    let packet = plan.evaluate(plan.spec().fps.sample(0), Some([3, 3]))?;
    assert_eq!(packet.viewport, [0.3, 0.3, 2.4, 2.4]);
    let field = readbacks.render(renderer, &packet)?;
    let mut blank = packet.clone();
    blank.surfaces.clear();
    let blank = readbacks.render(renderer, &blank)?;
    assert_eq!(
        changed_pixels(&field, &blank),
        9,
        "all 3x3 pixel centers lie within the fractional viewport [0.3, 0.3, 2.4, 2.4]; scissor rounding must preserve the last row and column"
    );
    println!("PASS fractional viewport preserves all covered pixel centers at 3x3 output");
    Ok(())
}

fn retained_frames(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let mut a = packet(empty_scene())?;
    a.background = [0.8, 0.1, 0.2, 1.0];
    let mut b = a.clone();
    b.background = [0.1, 0.7, 0.4, 1.0];
    // Both submissions complete before either readback: a retained frame is a snapshot.
    let first = renderer.render(&a)?;
    let second = renderer.render(&b)?;
    let first_pixels = readbacks.capture.read_rgba8(renderer, &first)?;
    let second_pixels = readbacks.capture.read_rgba8(renderer, &second)?;
    assert_eq!(first_pixels, readbacks.render(renderer, &a)?);
    assert_eq!(second_pixels, readbacks.render(renderer, &b)?);
    assert_eq!(changed_pixels(&first_pixels, &second_pixels), 128 * 128);
    println!("PASS two held GpuFrames retain distinct render results across later submissions");
    Ok(())
}

fn capture_device_ownership(renderer: &mut Renderer, options: &GpuOptions) -> Result<()> {
    let mut other = pollster::block_on(Renderer::new(options))?;
    let packet = packet(empty_scene())?;
    let first = renderer.render(&packet)?;
    let second = other.render(&packet)?;
    let mut capture = Capture::default();
    let expected = capture.read_rgba8(renderer, &first)?;
    let error = capture.read_rgba8(&other, &first).unwrap_err();
    assert!(error.to_string().contains("owns the frame's GPU device"));
    assert_eq!(capture.read_rgba8(renderer, &first)?, expected);
    // Equal-size transfers must replace the old device's staging allocation.
    let other_pixels = capture.read_rgba8(&other, &second)?;
    assert_eq!(capture.read_rgba8(&other, &second)?, other_pixels);
    assert_eq!(capture.read_rgba8(renderer, &first)?, expected);
    println!("PASS capture rejects foreign frames and safely switches staging-buffer devices");
    Ok(())
}

fn shuffled_reference(renderer: &mut Renderer, readbacks: &mut Readbacks) -> Result<()> {
    let start = Instant::now();
    let plan = CompiledPlan::compile(lemma::reference::scene(), Path::new("."))?;
    let compile_ms = start.elapsed().as_secs_f64() * 1000.0;
    let frames = [0, 120, 240, 479];
    let mut expected = Vec::new();
    let mut evaluation_ms = 0.0;
    for frame in frames {
        let start = Instant::now();
        let packet = plan.evaluate(plan.spec().fps.sample(frame), Some([320, 180]))?;
        evaluation_ms += start.elapsed().as_secs_f64() * 1000.0;
        expected.push(readbacks.render(renderer, &packet)?);
    }
    assert!(changed_pixels(&expected[0], &expected[3]) > 1000);
    let uploads = renderer.stats.field_uploads;
    for i in [3, 0, 2, 1, 0, 3] {
        let start = Instant::now();
        let packet = plan.evaluate(plan.spec().fps.sample(frames[i]), Some([320, 180]))?;
        evaluation_ms += start.elapsed().as_secs_f64() * 1000.0;
        let actual = readbacks.render(renderer, &packet)?;
        assert_eq!(
            actual, expected[i],
            "reference frame {} changed after shuffled requests",
            frames[i]
        );
    }
    assert_eq!(renderer.stats.field_uploads, uploads);
    println!(
        "PASS reference direct/repeated/shuffled seeks: byte-identical at frames {frames:?} (320x180)"
    );
    println!(
        "MEASURE reference compilation {compile_ms:.3} ms; 10 explicit evaluations {evaluation_ms:.3} ms"
    );
    Ok(())
}

#[test]
#[ignore = "requires an available GPU; software requires explicit LEMMA_GPU_ALLOW_SOFTWARE=1"]
fn rendered_gpu_contracts() -> Result<()> {
    let options = GpuOptions {
        adapter: std::env::var("LEMMA_GPU_ADAPTER").ok(),
        allow_software: std::env::var("LEMMA_GPU_ALLOW_SOFTWARE").as_deref() == Ok("1"),
        ..GpuOptions::default()
    };
    let start = Instant::now();
    let mut renderer =
        pollster::block_on(Renderer::new(&options)).context("initialize opt-in GPU contracts")?;
    let initialization_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("ADAPTER {}", serde_json::to_string(&renderer.adapter)?);
    println!("MEASURE adapter/device/pipeline initialization {initialization_ms:.3} ms");
    let mut readbacks = Readbacks::default();
    srgb_background(&mut renderer, &mut readbacks)?;
    text_alpha(&mut renderer, &mut readbacks)?;
    negative_glyph_bearings(&mut renderer, &mut readbacks)?;
    opaque_depth_and_labels(&mut renderer, &mut readbacks)?;
    spatial_strokes(&mut renderer, &mut readbacks)?;
    extreme_coordinate_strokes(&mut renderer, &mut readbacks)?;
    world_graphics_depth(&mut renderer, &mut readbacks)?;
    palette_legend(&mut renderer, &mut readbacks)?;
    model_transforms(&mut renderer, &mut readbacks)?;
    field_cache(&mut renderer, &mut readbacks)?;
    fractional_viewport(&mut renderer, &mut readbacks)?;
    retained_frames(&mut renderer, &mut readbacks)?;
    shuffled_reference(&mut renderer, &mut readbacks)?;
    capture_device_ownership(&mut renderer, &options)?;
    println!(
        "MEASURE {} measured frames: prepare/upload CPU {:.3} ms; record/submit CPU {:.3} ms; capture wait/readback {:.3} ms",
        readbacks.count,
        readbacks.prepare_ms,
        readbacks.record_and_submit_ms,
        readbacks.wait_and_readback_ms
    );
    println!("STATS {}", serde_json::to_string(&renderer.stats)?);
    Ok(())
}
