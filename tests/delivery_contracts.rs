//! Delivery validation must fail before touching a GPU or creating output files.
use lemma::{
    animation::tween,
    eval::CompiledPlan,
    export::{Delivery, RangeOptions, render_range},
    reference,
    render::gpu::GpuOptions,
};
use std::path::Path;

#[test]
fn invalid_export_ranges_and_sizes_do_not_create_output() {
    let plan = CompiledPlan::compile(reference::scene(), Path::new(".")).unwrap();
    let directory =
        std::env::temp_dir().join(format!("lemma-invalid-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    for (start, end, size, expected) in [
        (4, 4, [128, 72], "exclusive end frame"),
        (4, 3, [128, 72], "exclusive end frame"),
        (0, 1, [0, 72], "export size"),
        (0, 1, [128, 8193], "export size"),
    ] {
        let result = render_range(
            &plan,
            &GpuOptions::default(),
            &RangeOptions {
                start,
                end: Some(end),
                fps: None,
                size: Some(size),
            },
            Delivery::PngSequence {
                directory: &directory,
                overwrite: false,
            },
        );
        let error = result.unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(!directory.exists());
    }
}

#[test]
fn first_requested_sample_is_validated_before_creating_outputs_or_starting_encoder() {
    let mut scene = reference::scene();
    scene.camera.eye = tween(
        scene.camera.eye.at(0.0),
        scene.camera.target.at(0.0),
        0.0,
        1.0,
    );
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    assert!(plan.evaluate(plan.spec().fps.sample(0), None).is_ok());
    let directory =
        std::env::temp_dir().join(format!("lemma-invalid-first-sample-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let video = directory.join("video.mp4");
    for delivery in [
        Delivery::PngSequence {
            directory: &directory,
            overwrite: false,
        },
        Delivery::Mp4 {
            path: &video,
            ffmpeg: Path::new("/this/encoder/must/not/be/started"),
            overwrite: false,
        },
    ] {
        let error = render_range(
            &plan,
            &GpuOptions::default(),
            &RangeOptions {
                start: 60,
                end: Some(61),
                fps: None,
                size: Some([128, 72]),
            },
            delivery,
        )
        .unwrap_err();
        assert!(error.to_string().contains("degenerate view"), "{error:#}");
        assert!(!directory.exists());
    }
}

#[test]
fn invalid_video_parameters_do_not_publish_output_or_start_encoder() {
    let plan = CompiledPlan::compile(reference::scene(), Path::new(".")).unwrap();
    let directory =
        std::env::temp_dir().join(format!("lemma-invalid-video-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    for (filename, size, expected) in [
        ("odd.mp4", [127, 72], "positive even dimensions"),
        ("wrong.webm", [128, 72], "must use the .mp4 extension"),
    ] {
        let result = render_range(
            &plan,
            &GpuOptions::default(),
            &RangeOptions {
                start: 0,
                end: Some(1),
                fps: None,
                size: Some(size),
            },
            Delivery::Mp4 {
                path: &directory.join(filename),
                ffmpeg: Path::new("/this/encoder/must/not/be/started"),
                overwrite: false,
            },
        );
        let error = result.unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(!directory.exists());
    }
}
