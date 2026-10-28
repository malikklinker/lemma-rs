use lemma::{
    animation::tween,
    data::{FieldSource, ScalarField, SourceCache},
    eval::{CompiledPlan, Placement},
    reference,
    scene::{Anchor, MAX_STROKE_WIDTH, NodeSpec, Occlusion, TransformSpec},
    time::FrameRate,
};
use std::{path::Path, sync::Arc};

#[test]
fn attached_geometry_follows_rendered_triangles_across_direct_seeks() {
    let directory = std::env::temp_dir().join(format!(
        "lemma-eval-triangle-contract-{}",
        std::process::id()
    ));
    let field = ScalarField::new(2, 2, [0., 1., 0., 1.], vec![4., 0., 0., 8.]).unwrap();
    let manifest = field.save(&directory).unwrap();
    let mut scene = reference::scene();
    scene.fps = FrameRate::new(30, 1).unwrap();
    scene.assets[0].source = FieldSource::Saved { manifest };
    let height = tween(0.0, 2.0, 0.0, 2.0);
    let points = vec![[0.25, 0.25], [0.5, 0.5], [0.75, 0.75]];
    scene.nodes = vec![
        NodeSpec::Trace {
            id: "attached-trace".into(),
            field: "field".into(),
            points: points.clone(),
            height: height.clone(),
            lift: 0.25,
            width: 2.0,
            color: [1.; 4],
            reveal: 1.0.into(),
        },
        NodeSpec::Markers {
            id: "attached-markers".into(),
            field: "field".into(),
            points,
            height: height.clone(),
            lift: 0.25,
            radius: 0.1,
            colors: vec![[1.; 4]],
        },
        NodeSpec::Label {
            id: "attached-label".into(),
            text: "Probe".into(),
            anchor: Anchor::Field {
                field: "field".into(),
                point: [0.75, 0.75],
                height,
                lift: 0.25,
            },
            offset: [0., 0.],
            font_size: 12.0,
            color: [1.; 4],
            occlusion: Occlusion::Hide,
        },
    ];
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    let retained = Arc::clone(&plan.fields()["field"]);
    // Once compiled, this scene no longer needs its source files for seeking.
    std::fs::remove_dir_all(directory).unwrap();
    for (frame, scale) in [(60, 2.0), (0, 0.0), (30, 1.0), (60, 2.0), (0, 0.0)] {
        let packet = plan.evaluate(plan.spec().fps.sample(frame), None).unwrap();
        let expected = [
            [0.25, 2.0 * scale + 0.25, 0.25],
            [0.5, 0.25, 0.5],
            [0.75, 4.0 * scale + 0.25, 0.75],
        ];
        assert_eq!(packet.paths[0].points, expected);
        for (marker, expected) in packet.markers.iter().zip(expected) {
            assert_eq!(marker.position, expected);
        }
        let Placement::Projected { anchor, .. } = packet.graphics[0].placement else {
            panic!("field label must retain projected placement")
        };
        assert_eq!(anchor, expected[2]);
        assert!(Arc::ptr_eq(&retained, &plan.fields()["field"]));
    }
}

fn close(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
}

#[test]
fn model_transform_moves_all_world_content_and_preserves_screen_coordinates() {
    let mut scene = reference::scene();
    scene.model = TransformSpec {
        translation: [1., 2., 3.].into(),
        rotation_degrees: [0., 90., 0.].into(),
        scale: 2.0.into(),
    };
    let mut surface = scene.nodes[0].clone();
    if let NodeSpec::Surface { height, .. } = &mut surface {
        *height = 0.0.into();
    }
    scene.nodes = vec![
        surface,
        NodeSpec::Trace {
            id: "model-trace".into(),
            field: "field".into(),
            points: vec![[0., 0.], [1., 0.]],
            height: 0.0.into(),
            lift: 0.0,
            width: 3.0,
            color: [1.; 4],
            reveal: 1.0.into(),
        },
        NodeSpec::Polyline {
            id: "model-path".into(),
            points: vec![[0., 0., 0.], [1., 0., 0.]],
            width: 3.0,
            color: [1.; 4],
            reveal: 1.0.into(),
        },
        NodeSpec::Markers {
            id: "model-markers".into(),
            field: "field".into(),
            points: vec![[1., 0.]],
            height: 0.0.into(),
            lift: 0.0,
            radius: 0.5,
            colors: vec![[1.; 4]],
        },
        NodeSpec::Label {
            id: "model-world-label".into(),
            text: "Probe".into(),
            anchor: Anchor::World {
                position: [1., 0., 0.].into(),
            },
            offset: [4., 5.],
            font_size: 12.,
            color: [1.; 4],
            occlusion: Occlusion::Hide,
        },
        NodeSpec::Label {
            id: "model-field-label".into(),
            text: "Probe".into(),
            anchor: Anchor::Field {
                field: "field".into(),
                point: [1., 0.],
                height: 0.0.into(),
                lift: 0.0,
            },
            offset: [4., 5.],
            font_size: 12.,
            color: [1.; 4],
            occlusion: Occlusion::Hide,
        },
        NodeSpec::GraphicsPlane {
            id: "model-plane".into(),
            text: "Plane".into(),
            position: [1., 0., 0.].into(),
            right: [1., 0., 0.],
            up: [0., 1., 0.],
            width: 2.,
            font_size: 12.,
            color: [1.; 4],
        },
        NodeSpec::Text {
            id: "fixed-screen-text".into(),
            text: "Screen".into(),
            position: [3., 4.],
            font_size: 12.,
            color: [1.; 4],
            opacity: 1.0.into(),
        },
    ];
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    let packet = plan.evaluate(plan.spec().fps.sample(0), None).unwrap();
    close(
        packet.surfaces[0]
            .model
            .transform_point3([1., 0., 0.].into())
            .to_array(),
        [1., 2., 1.],
    );
    for path in &packet.paths {
        close(path.points[0], [1., 2., 3.]);
        close(path.points[1], [1., 2., 1.]);
        assert_eq!(path.width, 3.0);
    }
    close(packet.markers[0].position, [1., 2., 1.]);
    assert_eq!(packet.markers[0].radius, 1.0);
    for label in &packet.graphics[..2] {
        let Placement::Projected { anchor, offset, .. } = label.placement else {
            panic!("expected label")
        };
        close(anchor, [1., 2., 1.]);
        assert_eq!(offset, [4., 5.]);
        assert_eq!(label.font_size, 12.0);
    }
    let Placement::World {
        position,
        right,
        up,
        width,
    } = packet.graphics[2].placement
    else {
        panic!("expected world plane")
    };
    close(position, [1., 2., 1.]);
    close(right, [0., 0., -1.]);
    close(up, [0., 1., 0.]);
    assert_eq!(width, 4.0);
    let Placement::Screen { position } = packet.graphics[3].placement else {
        panic!("expected screen text")
    };
    assert_eq!(position, [3., 4.]);
}

#[test]
fn model_edits_reuse_numerical_results_and_invalid_scales_are_actionable() {
    let mut cache = SourceCache::default();
    let first =
        CompiledPlan::compile_with_cache(reference::scene(), Path::new("."), &mut cache).unwrap();
    let mut scene = first.spec().clone();
    scene.model.translation = [1., 2., 3.].into();
    scene.model.scale = 2.0.into();
    let second = CompiledPlan::compile_with_cache(scene, Path::new("."), &mut cache).unwrap();
    assert!(Arc::ptr_eq(
        &first.fields()["field"],
        &second.fields()["field"]
    ));
    assert_eq!(cache.stats().synthetic_computations, 1);
    for scale in [0.0, -1.0] {
        let mut scene = reference::scene();
        scene.model.scale = scale.into();
        let error = CompiledPlan::compile(scene, Path::new(".")).err().unwrap();
        assert!(format!("{error:#}").contains("model.scale"));
    }
    let mut scene = reference::scene();
    scene.model.scale = f32::MAX.into();
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    let error = plan.evaluate(plan.spec().fps.sample(0), None).unwrap_err();
    assert!(format!("{error:#}").contains("node"));
}

#[test]
fn unsupported_translucent_paths_report_the_stable_node_id() {
    for trace in [false, true] {
        let mut scene = reference::scene();
        let id = if trace {
            "faded-trace"
        } else {
            "faded-polyline"
        };
        scene.nodes = vec![if trace {
            NodeSpec::Trace {
                id: id.into(),
                field: "field".into(),
                points: vec![[0., 0.], [1., 1.]],
                height: 1.0.into(),
                lift: 0.0,
                width: 2.0,
                color: [1., 1., 1., 0.5],
                reveal: 1.0.into(),
            }
        } else {
            NodeSpec::Polyline {
                id: id.into(),
                points: vec![[0., 0., 0.], [1., 0., 1.]],
                width: 2.0,
                color: [1., 1., 1., 0.5],
                reveal: 1.0.into(),
            }
        }];
        let error = CompiledPlan::compile(scene, Path::new(".")).err().unwrap();
        let message = format!("{error:#}");
        assert!(message.contains(id), "{message}");
        assert!(message.contains("translucent"), "{message}");
        assert!(message.contains("opaque color"), "{message}");
    }
}

#[test]
fn path_width_limits_are_checked_before_and_after_output_scaling() {
    for trace in [false, true] {
        let mut scene = reference::scene();
        let id = if trace { "wide-trace" } else { "wide-polyline" };
        let node = |width| {
            if trace {
                NodeSpec::Trace {
                    id: id.into(),
                    field: "field".into(),
                    points: vec![[0., 0.], [1., 1.]],
                    height: 1.0.into(),
                    lift: 0.0,
                    width,
                    color: [1.; 4],
                    reveal: 1.0.into(),
                }
            } else {
                NodeSpec::Polyline {
                    id: id.into(),
                    points: vec![[0., 0., 0.], [1., 0., 1.]],
                    width,
                    color: [1.; 4],
                    reveal: 1.0.into(),
                }
            }
        };
        for width in [0.0, -1.0, MAX_STROKE_WIDTH + 1.0, f32::MAX] {
            scene.nodes = vec![node(width)];
            let error = CompiledPlan::compile(scene.clone(), Path::new("."))
                .err()
                .unwrap();
            let message = format!("{error:#}");
            assert!(message.contains(id), "{message}");
            assert!(message.contains("(0, 32768] pixels"), "{message}");
        }
        scene.nodes = vec![node(MAX_STROKE_WIDTH)];
        let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
        let packet = plan.evaluate(plan.spec().fps.sample(0), None).unwrap();
        assert_eq!(packet.paths[0].width, MAX_STROKE_WIDTH);
        let error = plan
            .evaluate(plan.spec().fps.sample(0), Some([2560, 1440]))
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains(id), "{message}");
        assert!(
            message.contains("reduce width or output scale"),
            "{message}"
        );
    }
}

#[test]
fn marker_projection_overflow_reports_the_stable_node_id() {
    let mut scene = reference::scene();
    scene.camera.eye = [0., 0., 5.].into();
    scene.camera.target = [0.; 3].into();
    scene.camera.up = [0., 1., 0.];
    scene.camera.fov_y_degrees = 2.0;
    scene.nodes = vec![NodeSpec::Markers {
        id: "magnified-marker".into(),
        field: "field".into(),
        points: vec![[0., 0.]],
        height: 0.0.into(),
        lift: 0.0,
        radius: 1e38,
        colors: vec![[1.; 4]],
    }];
    let plan = CompiledPlan::compile(scene.clone(), Path::new(".")).unwrap();
    let error = plan.evaluate(scene.fps.sample(0), None).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("magnified-marker"), "{message}");
    assert!(message.contains("projected marker extent"), "{message}");
    assert!(message.contains("reduce marker radius"), "{message}");

    // Being outside the camera frustum is valid; only non-finite geometry
    // should fail evaluation, including when seeking directly to later frames.
    if let NodeSpec::Markers { radius, .. } = &mut scene.nodes[0] {
        *radius = 100.0;
    }
    let plan = CompiledPlan::compile(scene.clone(), Path::new(".")).unwrap();
    for frame in [60, 0, 30, 60] {
        let packet = plan.evaluate(scene.fps.sample(frame), None).unwrap();
        assert_eq!(packet.markers[0].radius, 100.0);
    }
}
