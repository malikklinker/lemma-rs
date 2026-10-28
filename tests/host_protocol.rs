use lemma::{
    data::{FieldSource, ScalarField, SourceCache, SourcePolicy},
    eval::CompiledPlan,
    host::{Host, serve},
    reference,
    render::gpu::GpuOptions,
    scene::{MAX_STROKE_WIDTH, NodeSpec},
};
use serde_json::{Value, json};
use std::{io::Cursor, path::Path, sync::Arc};

fn request(id: u64, operation: &str) -> Value {
    json!({"version":1,"id":id,"op":operation})
}

fn load(host: &mut Host, id: u64, scene: Value) -> Value {
    let mut message = request(id, "load-scene");
    message["scene"] = scene;
    host.process(message)
}

#[test]
fn protocol_recovers_from_invalid_messages_and_preserves_ids() {
    let scene = serde_json::to_value(reference::scene()).unwrap();
    let lines = [
        "{broken".to_owned(),
        json!({"version":7,"id":"future","op":"info"}).to_string(),
        json!({"version":1,"id":null,"op":"info"}).to_string(),
        json!({"version":1,"id":8,"op":"info","unexpected":true}).to_string(),
        json!({"version":1,"id":"load","op":"load-scene","scene":scene}).to_string(),
        json!({"version":1,"id":9,"op":"inspect-sample","frame":90}).to_string(),
        json!({"version":1,"id":"bye","op":"shutdown"}).to_string(),
        json!({"version":1,"id":"ignored","op":"info"}).to_string(),
    ];
    let mut output = Vec::new();
    serve(
        Cursor::new(lines.join("\n")),
        &mut output,
        GpuOptions::default(),
    )
    .unwrap();
    let responses: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 7);
    assert_eq!(responses[0]["error"]["code"], "invalid-json");
    assert_eq!(responses[1]["id"], "future");
    assert_eq!(responses[1]["error"]["code"], "unsupported-version");
    assert_eq!(responses[2]["id"], Value::Null);
    assert_eq!(responses[2]["error"]["code"], "invalid-id");
    assert_eq!(responses[3]["id"], 8);
    assert_eq!(responses[3]["error"]["code"], "invalid-request");
    assert_eq!(responses[4]["id"], "load");
    assert_eq!(responses[4]["result"]["revision"], 1);
    assert_eq!(responses[5]["result"]["sample"]["frame"], 90);
    assert_eq!(responses[6]["id"], "bye");
    assert_eq!(responses[6]["result"]["shutdown"], true);
}

#[test]
fn invalid_reload_keeps_last_valid_scene_without_initializing_gpu() {
    let mut host = Host::new(GpuOptions::default());
    let scene = serde_json::to_value(reference::scene()).unwrap();
    assert_eq!(load(&mut host, 1, scene.clone())["ok"], true);
    let mut inspect = request(2, "inspect-sample");
    inspect["frame"] = 180.into();
    let baseline = host.process(inspect.clone());

    let mut invalid = scene.clone();
    invalid["nodes"][0]["field"] = "missing-field".into();
    let failure = load(&mut host, 3, invalid);
    assert_eq!(failure["ok"], false);
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing-field")
    );
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains(scene["nodes"][0]["id"].as_str().unwrap())
    );
    assert_eq!(host.process(inspect.clone())["result"], baseline["result"]);

    let mut unknown = scene.clone();
    unknown["nodes"][0]["unsupported_feature"] = true.into();
    let failure = load(&mut host, 4, unknown);
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unsupported_feature")
    );
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains(scene["nodes"][0]["id"].as_str().unwrap())
    );

    let mut degenerate = scene.clone();
    degenerate["camera"]["eye"] = degenerate["camera"]["target"].clone();
    assert_eq!(load(&mut host, 5, degenerate)["ok"], false);
    let mut no_computation = request(6, "load-scene");
    no_computation["scene"] = scene;
    no_computation["policy"] = "require-saved".into();
    assert_eq!(host.process(no_computation)["ok"], false);
    assert_eq!(host.process(inspect)["result"], baseline["result"]);
    let info = host.process(request(7, "info"));
    assert_eq!(info["result"]["revision"], 1);
    assert_eq!(info["result"]["adapter"], Value::Null);
    assert_eq!(info["result"]["cache"]["synthetic_computations"], 1);
}

#[test]
fn numerical_identity_survives_camera_palette_and_annotation_edits() {
    let mut cache = SourceCache::default();
    let scene = reference::scene();
    let first =
        CompiledPlan::compile_with_cache(scene.clone(), Path::new("."), &mut cache).unwrap();
    let field_id = scene.assets[0].id.clone();
    let mut edited = scene.clone();
    edited.camera.fov_y_degrees += 2.0;
    for node in &mut edited.nodes {
        match node {
            NodeSpec::Surface { palette, .. } => palette.stops[0] = [0.2, 0.3, 0.4, 1.0],
            NodeSpec::Text { text, .. } => *text = "A revised annotation".into(),
            _ => {}
        }
    }
    let second = CompiledPlan::compile_with_cache(edited, Path::new("."), &mut cache).unwrap();
    assert!(Arc::ptr_eq(
        &first.fields()[&field_id],
        &second.fields()[&field_id]
    ));
    assert_eq!(cache.stats().synthetic_computations, 1);
    assert_eq!(cache.stats().hits, 1);

    let mut numerical = scene;
    if let FieldSource::Synthetic { resolution, .. } = &mut numerical.assets[0].source {
        resolution[0] += 1;
    }
    let third = CompiledPlan::compile_with_cache(numerical, Path::new("."), &mut cache).unwrap();
    assert!(!Arc::ptr_eq(
        &first.fields()[&field_id],
        &third.fields()[&field_id]
    ));
    assert_ne!(
        first.fields()[&field_id].content_hash(),
        third.fields()[&field_id].content_hash()
    );
    assert_eq!(cache.stats().synthetic_computations, 2);
    let mut seeded = SourceCache::default();
    seeded
        .seed(
            &second.spec().assets[0].source,
            second.fields()[&field_id].clone(),
        )
        .unwrap();
    let reused = seeded
        .resolve(
            &second.spec().assets[0].source,
            Path::new("."),
            SourcePolicy::OnDemand,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&reused, &second.fields()[&field_id]));
    assert_eq!(seeded.stats().synthetic_computations, 0);
    assert!(
        seeded
            .seed(
                &third.spec().assets[0].source,
                second.fields()[&field_id].clone()
            )
            .is_err()
    );
}

#[test]
fn camera_and_resolved_node_overflows_are_actionable_errors() {
    let mut scene = reference::scene();
    scene.camera.near = 3e38;
    scene.camera.far = 3.4e38;
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    assert!(
        plan.evaluate(plan.spec().fps.sample(0), None)
            .unwrap_err()
            .to_string()
            .contains("camera")
    );

    let mut scene = reference::scene();
    scene.nodes = vec![NodeSpec::Polyline {
        id: "overflowing-stroke".into(),
        points: vec![[0., 0., 0.], [1., 0., 0.]],
        width: MAX_STROKE_WIDTH,
        color: [1.; 4],
        reveal: 1.0.into(),
    }];
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    assert!(
        plan.evaluate(plan.spec().fps.sample(0), Some([2560, 1440]))
            .unwrap_err()
            .to_string()
            .contains("overflowing-stroke")
    );

    let directory =
        std::env::temp_dir().join(format!("lemma-host-overflow-{}", std::process::id()));
    let field = ScalarField::new(2, 2, [-3., 3., -3., 3.], vec![2.; 4]).unwrap();
    let manifest = field.save(&directory).unwrap();
    let mut scene = reference::scene();
    scene.assets[0].source = FieldSource::Saved { manifest };
    scene.nodes.truncate(1);
    if let NodeSpec::Surface { height, .. } = &mut scene.nodes[0] {
        *height = f32::MAX.into();
    }
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    assert!(
        plan.evaluate(plan.spec().fps.sample(0), None)
            .unwrap_err()
            .to_string()
            .contains("surface")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unsupported_text_reload_and_scaled_text_retain_valid_host_scene() {
    let mut host = Host::new(GpuOptions::default());
    let scene = serde_json::to_value(reference::scene()).unwrap();
    assert_eq!(load(&mut host, 1, scene.clone())["ok"], true);
    let mut unsupported = scene.clone();
    unsupported["nodes"][3]["text"] = "Unsupported: 🧪".into();
    let response = load(&mut host, 2, unsupported);
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("eyebrow")
    );
    let mut oversize = scene;
    oversize["nodes"][3]["text"] = "W".repeat(4096).into();
    let response = load(&mut host, 3, oversize);
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("eyebrow")
    );
    assert_eq!(host.process(request(4, "info"))["result"]["revision"], 1);

    let mut scene = reference::scene();
    scene.nodes = vec![NodeSpec::Text {
        id: "scaled-text".into(),
        text: "W".repeat(40),
        position: [0., 0.],
        font_size: 100.,
        color: [1.; 4],
        opacity: 1.0.into(),
    }];
    let plan = CompiledPlan::compile(scene, Path::new(".")).unwrap();
    let error = plan
        .evaluate(plan.spec().fps.sample(0), Some([8192, 8192]))
        .unwrap_err();
    assert!(format!("{error:#}").contains("scaled-text"));
}

#[test]
fn saved_source_cache_revalidates_payload_and_provenance_on_each_reload() {
    let directory = std::env::temp_dir().join(format!("lemma-host-saved-{}", std::process::id()));
    let field = ScalarField::synthetic([3, 2], [-1.0, 1.0, -1.0, 1.0]).unwrap();
    let manifest = field.save(&directory).unwrap();
    let source = FieldSource::Saved {
        manifest: manifest.clone(),
    };
    let mut cache = SourceCache::default();
    let first = cache
        .resolve(&source, Path::new("."), SourcePolicy::RequireSaved)
        .unwrap();
    let second = cache
        .resolve(&source, Path::new("."), SourcePolicy::RequireSaved)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.stats().synthetic_computations, 0);
    assert_eq!(cache.stats().saved_validations, 2);
    let payload = directory.join(format!("{}.f32", field.content_hash()));
    std::fs::write(&payload, [0u8; 24]).unwrap();
    assert!(
        cache
            .resolve(&source, Path::new("."), SourcePolicy::RequireSaved)
            .is_err()
    );
    field.save(&directory).unwrap();
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    metadata["provenance"]["request_hash"] = "incorrect".into();
    std::fs::write(&manifest, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let error = cache
        .resolve(&source, Path::new("."), SourcePolicy::RequireSaved)
        .unwrap_err();
    assert!(error.to_string().contains("provenance"));

    let replacement = ScalarField::synthetic([4, 3], [-1.0, 1.0, -1.0, 1.0]).unwrap();
    let replacement_manifest = replacement.save(&directory).unwrap();
    std::fs::copy(replacement_manifest, &manifest).unwrap();
    let refreshed = cache
        .resolve(&source, Path::new("."), SourcePolicy::RequireSaved)
        .unwrap();
    assert_ne!(first.content_hash(), refreshed.content_hash());
    assert!(!Arc::ptr_eq(&first, &refreshed));
    std::fs::remove_dir_all(directory).unwrap();
}
