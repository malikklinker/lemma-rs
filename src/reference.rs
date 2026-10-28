//! The acceptance scene is ordinary SceneSpec, also exported for TSX authoring.
use crate::{
    animation::{Animated, tween},
    data::FieldSource,
    scene::*,
    time::FrameRate,
};

pub fn scene() -> SceneSpec {
    let height = tween(0.0, 1.55, 1.4, 5.8);
    let palette = Palette {
        range: [-0.85, 0.95],
        ..Palette::default()
    };
    let points: Vec<_> = (0..241)
        .map(|i| {
            let t = i as f32 / 240.0;
            [-2.8 + 5.6 * t, 0.9 * (t * std::f32::consts::TAU).sin()]
        })
        .collect();
    let marker_points = [24, 68, 113, 158, 207].map(|i| points[i]);
    let muted = [0.52, 0.62, 0.71, 1.0];
    let bright = [0.94, 0.97, 0.99, 1.0];
    SceneSpec {
        version: 1,
        size: [1280, 720],
        fps: FrameRate::default(),
        duration: 8.0,
        background: [0.027, 0.047, 0.075, 1.0],
        camera: CameraSpec {
            viewport: Some([0., 148., 1280., 452.]),
            eye: tween([0.0, 10.3, 0.12], [7.5, 6.3, 8.3], 1.0, 6.4),
            target: Animated::Constant([0.15, 0.0, 0.5]),
            up: [0.0, 1.0, 0.0],
            fov_y_degrees: 50.0,
            near: 0.1,
            far: 100.0,
        },
        model: TransformSpec {
            rotation_degrees: tween([0., 0., 0.], [0., 12., 0.], 2.0, 6.4),
            scale: tween(1.0, 0.96, 2.0, 6.4),
            ..TransformSpec::default()
        },
        assets: vec![FieldAsset {
            id: "field".into(),
            source: FieldSource::Synthetic {
                resolution: [129, 129],
                bounds: [-3., 3., -3., 3.],
            },
        }],
        nodes: vec![
            NodeSpec::Surface {
                id: "surface".into(),
                field: "field".into(),
                height: height.clone(),
                palette: palette.clone(),
                grid: 0.17,
            },
            NodeSpec::Trace {
                id: "trace".into(),
                field: "field".into(),
                points,
                height: height.clone(),
                lift: 0.055,
                width: 3.5,
                color: [0.94, 0.98, 1.0, 1.0],
                reveal: tween(0.0, 1.0, 0.1, 3.8),
            },
            NodeSpec::Markers {
                id: "samples".into(),
                field: "field".into(),
                points: marker_points.into(),
                height: height.clone(),
                lift: 0.12,
                radius: 0.075,
                colors: vec![
                    [1.0, 0.53, 0.31, 1.0],
                    [1.0, 0.76, 0.35, 1.0],
                    [0.95, 0.97, 0.99, 1.0],
                    [0.39, 0.86, 0.83, 1.0],
                    [0.48, 0.69, 1.0, 1.0],
                ],
            },
            NodeSpec::Text {
                id: "eyebrow".into(),
                text: "L E M M A   /   S T U D Y  0 0 1".into(),
                position: [54., 30.],
                font_size: 12.,
                color: [0.37, 0.77, 0.74, 1.0],
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "title".into(),
                text: "A field, given form".into(),
                position: [51., 57.],
                font_size: 37.,
                color: bright,
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "equation".into(),
                text: "(x, z)  →  (x, h · f(x, z), z)".into(),
                position: [54., 108.],
                font_size: 18.,
                color: muted,
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "section".into(),
                text: "SAME SAMPLES. NEW PERSPECTIVE.".into(),
                position: [54., 616.],
                font_size: 12.,
                color: muted,
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "caption".into(),
                text: "A flat scalar map becomes a surface as height and camera evolve.".into(),
                position: [54., 641.],
                font_size: 16.,
                color: [0.76, 0.81, 0.85, 1.],
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "legend-title".into(),
                text: "FIELD VALUE".into(),
                position: [1020., 56.],
                font_size: 11.,
                color: muted,
                opacity: 1.0.into(),
            },
            NodeSpec::Legend {
                id: "legend".into(),
                position: [1020., 81.],
                size: [200., 9.],
                palette,
            },
            NodeSpec::Text {
                id: "legend-min".into(),
                text: "−0.85".into(),
                position: [1020., 101.],
                font_size: 12.,
                color: muted,
                opacity: 1.0.into(),
            },
            NodeSpec::Text {
                id: "legend-max".into(),
                text: "+0.95".into(),
                position: [1183., 101.],
                font_size: 12.,
                color: muted,
                opacity: 1.0.into(),
            },
            NodeSpec::Label {
                id: "probe-label".into(),
                text: "sample 03".into(),
                anchor: Anchor::Field {
                    field: "field".into(),
                    point: marker_points[2],
                    height,
                    lift: 0.24,
                },
                offset: [14., -28.],
                font_size: 15.,
                color: bright,
                occlusion: Occlusion::Hide,
            },
            NodeSpec::GraphicsPlane {
                id: "world-equation".into(),
                text: "h · f(x, z)".into(),
                position: [0.8, 1.8, -2.6].into(),
                right: [1., 0., 0.],
                up: [0., 1., 0.],
                width: 1.7,
                font_size: 60.,
                color: [0.57, 0.86, 0.83, 0.9],
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_surface_stays_inside_its_viewport_throughout_the_shot() {
        let plan = CompiledPlan::compile(scene(), Path::new(".")).unwrap();
        let mut maximum = (0.0, String::new());
        for frame in 0..plan.spec().fps.frame_count(plan.spec().duration).unwrap() {
            let packet = plan.evaluate(plan.spec().fps.sample(frame), None).unwrap();
            let surface = &packet.surfaces[0];
            let transform = packet.view_projection * surface.model;
            for row in 0..surface.field.height {
                for col in 0..surface.field.width {
                    let [x, z] = surface.field.position(col, row);
                    let y = surface.field.values[(row * surface.field.width + col) as usize]
                        * surface.height;
                    let clip = transform * glam::Vec4::new(x, y, z, 1.0);
                    let ndc = clip.truncate() / clip.w;
                    assert!(clip.w > 0.0, "surface must remain in front of camera");
                    let extent = ndc.x.abs().max(ndc.y.abs());
                    if extent > maximum.0 {
                        maximum = (
                            extent,
                            format!("frame {frame}, vertex ({col}, {row}), {ndc:?}"),
                        );
                    }
                }
            }
        }
        assert!(
            maximum.0 <= 0.98,
            "surface clips viewport margin: {}",
            maximum.1
        );
    }
    use crate::eval::CompiledPlan;
    use std::path::Path;
    #[test]
    fn direct_seeks_match_shuffled_evaluation_and_share_data() {
        let plan = CompiledPlan::compile(scene(), Path::new(".")).unwrap();
        let fps = plan.spec().fps;
        let expected: Vec<_> = (0..12)
            .map(|i| {
                let p = plan.evaluate(fps.sample(i * 37), None).unwrap();
                plan.inspect(&p)
            })
            .collect();
        for i in [11, 0, 8, 3, 10, 1, 6, 9, 4, 7, 2, 5] {
            let packet = plan.evaluate(fps.sample(i * 37), None).unwrap();
            assert_eq!(expected[i as usize], plan.inspect(&packet));
            assert!(std::sync::Arc::ptr_eq(
                &packet.surfaces[0].field,
                &plan.fields()["field"]
            ));
        }
    }
    #[test]
    fn scene_roundtrips_and_reports_bad_references() {
        let json = serde_json::to_string(&scene()).unwrap();
        let spec: SceneSpec = serde_json::from_str(&json).unwrap();
        assert!(CompiledPlan::compile(spec, Path::new(".")).is_ok());
        let mut bad = scene();
        if let NodeSpec::Surface { field, .. } = &mut bad.nodes[0] {
            *field = "missing".into();
        }
        let err = CompiledPlan::compile(bad, Path::new(".")).err().unwrap();
        assert!(format!("{err:#}").contains("surface"));
    }
}
