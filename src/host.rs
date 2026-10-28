//! Persistent, versioned JSON-lines control host. Standard output is protocol
//! only; scene loading and inspection do not initialize a graphics device.
use crate::{
    data::{SourceCache, SourcePolicy},
    eval::{CompiledPlan, parse_scene, read_scene},
    render::{
        Renderer,
        capture::{Capture, write_png},
        gpu::GpuOptions,
    },
    scene::SceneSpec,
    time::FrameRate,
};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_REQUEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
enum Command {
    LoadScene {
        scene: Value,
        #[serde(default = "current_directory")]
        base_dir: PathBuf,
        #[serde(default)]
        policy: SourcePolicy,
    },
    LoadPath {
        path: PathBuf,
        #[serde(default)]
        policy: SourcePolicy,
    },
    InspectSample {
        frame: u64,
        fps: Option<FrameRate>,
        size: Option<[u32; 2]>,
    },
    RenderSample {
        frame: u64,
        fps: Option<FrameRate>,
        size: Option<[u32; 2]>,
        output: PathBuf,
    },
    Info {},
    Shutdown {},
}

fn current_directory() -> PathBuf {
    PathBuf::from(".")
}

struct Diagnostic {
    code: &'static str,
    message: String,
}
impl Diagnostic {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn from_error(code: &'static str, error: anyhow::Error) -> Self {
        Self::new(code, format!("{error:#}"))
    }
}
type HostResult<T> = std::result::Result<T, Diagnostic>;

/// A host owns one numerical cache and, after its first render request, one GPU
/// renderer. Failed reloads never replace the last successfully compiled plan.
pub struct Host {
    gpu_options: GpuOptions,
    cache: SourceCache,
    plan: Option<CompiledPlan>,
    renderer: Option<Renderer>,
    capture: Capture,
    revision: u64,
    shutdown: bool,
}

impl Host {
    pub fn new(gpu_options: GpuOptions) -> Self {
        Self {
            gpu_options,
            cache: SourceCache::default(),
            plan: None,
            renderer: None,
            capture: Capture::default(),
            revision: 0,
            shutdown: false,
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Process one parsed JSON envelope; errors are values so clients can repair
    /// their scene and continue using the same process and device.
    pub fn process(&mut self, request: Value) -> Value {
        let id = request
            .get("id")
            .filter(|id| valid_id(id))
            .cloned()
            .unwrap_or(Value::Null);
        match self.dispatch(request) {
            Ok(result) => json!({"version":1,"id":id,"ok":true,"result":result}),
            Err(error) => error_response(id, error),
        }
    }

    fn dispatch(&mut self, mut request: Value) -> HostResult<Value> {
        let object = request
            .as_object_mut()
            .ok_or_else(|| Diagnostic::new("invalid-request", "request must be a JSON object"))?;
        if object.remove("version") != Some(json!(1)) {
            return Err(Diagnostic::new(
                "unsupported-version",
                "request version must be integer 1",
            ));
        }
        if !object.remove("id").as_ref().is_some_and(valid_id) {
            return Err(Diagnostic::new(
                "invalid-id",
                "request id must be a string or nonnegative integer",
            ));
        }
        let command: Command = serde_json::from_value(request)
            .map_err(|error| Diagnostic::new("invalid-request", error.to_string()))?;
        if self.shutdown {
            return Err(Diagnostic::new(
                "host-shutdown",
                "host has already shut down",
            ));
        }
        match command {
            Command::LoadScene {
                scene,
                base_dir,
                policy,
            } => {
                let spec =
                    parse_scene(scene).map_err(|e| Diagnostic::from_error("scene-error", e))?;
                self.load(spec, &base_dir, policy)
            }
            Command::LoadPath { path, policy } => {
                let spec =
                    read_scene(&path).map_err(|e| Diagnostic::from_error("scene-error", e))?;
                self.load(spec, path.parent().unwrap_or(Path::new(".")), policy)
            }
            Command::InspectSample { frame, fps, size } => {
                let plan = self.plan.as_ref().ok_or_else(no_scene)?;
                let packet = plan
                    .evaluate(fps.unwrap_or(plan.spec().fps).sample(frame), size)
                    .map_err(|e| Diagnostic::from_error("evaluation-error", e))?;
                let mut result = plan.inspect(&packet);
                result["revision"] = self.revision.into();
                Ok(result)
            }
            Command::RenderSample {
                frame,
                fps,
                size,
                output,
            } => self.render(frame, fps, size, &output),
            Command::Info {} => Ok(json!({
                "protocol_version":1, "revision":self.revision, "scene_loaded":self.plan.is_some(),
                "scene":self.plan.as_ref().map(|plan| self.describe(plan)),
                "adapter":self.renderer.as_ref().map(|renderer| &renderer.adapter),
                "renderer":self.renderer.as_ref().map(|renderer| &renderer.stats),
                "cache":self.cache.stats(),
                "operations":["load-scene","load-path","inspect-sample","render-sample","info","shutdown"]
            })),
            Command::Shutdown {} => {
                self.shutdown = true;
                Ok(json!({"shutdown":true}))
            }
        }
    }

    fn load(&mut self, spec: SceneSpec, base: &Path, policy: SourcePolicy) -> HostResult<Value> {
        let start = Instant::now();
        let plan = CompiledPlan::compile_with_policy(spec, base, &mut self.cache, policy)
            .map_err(|e| Diagnostic::from_error("scene-error", e))?;
        // Reject an immediately unusable camera before replacing the valid plan.
        plan.evaluate(plan.spec().fps.sample(0), None)
            .map_err(|e| Diagnostic::from_error("scene-error", e))?;
        self.plan = Some(plan);
        self.revision += 1;
        let mut result = self.describe(self.plan.as_ref().unwrap());
        result["revision"] = self.revision.into();
        result["cache"] = serde_json::to_value(self.cache.stats()).unwrap();
        result["compile_ms"] = milliseconds(start).into();
        Ok(result)
    }

    fn describe(&self, plan: &CompiledPlan) -> Value {
        json!({
            "size":plan.spec().size,"fps":plan.spec().fps,"duration":plan.spec().duration,
            "nodes":plan.spec().nodes.len(),
            "assets":plan.fields().iter().map(|(id, field)| json!({
                "id":id,"content_hash":field.content_hash,"shape":[field.height,field.width],"range":field.range
            })).collect::<Vec<_>>()
        })
    }

    fn render(
        &mut self,
        frame: u64,
        fps: Option<FrameRate>,
        size: Option<[u32; 2]>,
        output: &Path,
    ) -> HostResult<Value> {
        let start = Instant::now();
        let plan = self.plan.as_ref().ok_or_else(no_scene)?;
        let packet = plan
            .evaluate(fps.unwrap_or(plan.spec().fps).sample(frame), size)
            .map_err(|e| Diagnostic::from_error("evaluation-error", e))?;
        let evaluate_ms = milliseconds(start);
        let initialize = Instant::now();
        let initialize_ms = if self.renderer.is_none() {
            let renderer = pollster::block_on(Renderer::new(&self.gpu_options))
                .map_err(|e| Diagnostic::from_error("render-error", e))?;
            self.renderer = Some(renderer);
            milliseconds(initialize)
        } else {
            0.0
        };
        let renderer = self.renderer.as_mut().unwrap();
        let gpu_frame = renderer
            .render(&packet)
            .map_err(|e| Diagnostic::from_error("render-error", e))?;
        let readback = Instant::now();
        let pixels = self
            .capture
            .read_rgba8(renderer, &gpu_frame)
            .map_err(|e| Diagnostic::from_error("render-error", e))?;
        let readback_ms = milliseconds(readback);
        let encode = Instant::now();
        write_png(output, packet.size, &pixels)
            .map_err(|e| Diagnostic::from_error("output-error", e))?;
        Ok(json!({
            "revision":self.revision,"output":output,"sample":packet.sample,"size":packet.size,
            "adapter":renderer.adapter,"renderer":renderer.stats,"cache":self.cache.stats(),
            "timing_ms":{"evaluate":evaluate_ms,"initialize":initialize_ms,
                "readback_including_gpu_wait":readback_ms,"png":milliseconds(encode),"total":milliseconds(start)}
        }))
    }
}

fn no_scene() -> Diagnostic {
    Diagnostic::new("no-scene", "load a valid scene before requesting a sample")
}
fn valid_id(id: &Value) -> bool {
    id.is_string() || id.as_u64().is_some()
}
fn milliseconds(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn error_response(id: Value, error: Diagnostic) -> Value {
    json!({"version":1,"id":id,"ok":false,"error":{"code":error.code,"message":error.message}})
}

/// Serve until EOF or a successful shutdown request. One flushed response is
/// written per nonempty input line, including malformed JSON and oversized lines.
pub fn serve(
    mut reader: impl BufRead,
    mut writer: impl Write,
    gpu_options: GpuOptions,
) -> Result<()> {
    let mut host = Host::new(gpu_options);
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = reader
            .by_ref()
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        let response = if line.len() as u64 > MAX_REQUEST_BYTES {
            if !line.ends_with(b"\n") {
                reader.skip_until(b'\n')?;
            }
            error_response(
                Value::Null,
                Diagnostic::new(
                    "request-too-large",
                    "JSON request exceeds 64 MiB; reference binary assets for dense data",
                ),
            )
        } else if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        } else {
            match serde_json::from_slice(&line) {
                Ok(request) => host.process(request),
                Err(error) => error_response(
                    Value::Null,
                    Diagnostic::new("invalid-json", error.to_string()),
                ),
            }
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if host.is_shutdown() {
            break;
        }
    }
    Ok(())
}
