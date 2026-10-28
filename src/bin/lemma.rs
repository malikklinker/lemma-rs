use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use lemma::{
    data::{FieldSource, SourceCache, SourcePolicy},
    eval::{CompiledPlan, read_scene},
    export::{Delivery, RangeOptions, render_range},
    reference,
    render::{
        Renderer,
        capture::{Capture, write_png},
        gpu::{self, Backend, GpuOptions},
        preview::{self, PreviewOptions},
    },
    scene::SceneSpec,
    time::FrameRate,
};
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Parser)]
#[command(
    version,
    about = "Motion graphics from numerical data. Explicit samples, one renderer."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enumerate actual adapters and capabilities; does not select a fallback.
    Adapters {
        #[arg(long, value_enum, default_value = "vulkan")]
        backend: Backend,
    },
    /// Render one explicit sample to an sRGB PNG.
    RenderFrame {
        #[command(flatten)]
        scene: SceneArgs,
        #[command(flatten)]
        gpu: GpuArgs,
        #[arg(long, default_value_t = 0)]
        frame: u64,
        #[arg(long, default_value = "out/frame.png")]
        output: PathBuf,
    },
    /// Render a half-open range [start, end) to a PNG sequence.
    RenderRange {
        #[command(flatten)]
        scene: SceneArgs,
        #[command(flatten)]
        gpu: GpuArgs,
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long, default_value = "out/frames")]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Stream frames with bounded memory to FFmpeg/H.264 MP4.
    Export {
        #[command(flatten)]
        scene: SceneArgs,
        #[command(flatten)]
        gpu: GpuArgs,
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long, default_value = "out/reference.mp4")]
        output: PathBuf,
        #[arg(long, default_value = "ffmpeg")]
        ffmpeg: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Open a native GPU preview. Space play/pause, arrows step, L loop, R reload.
    Preview {
        #[command(flatten)]
        scene: SceneArgs,
        #[command(flatten)]
        gpu: GpuArgs,
        #[arg(long, default_value_t = 0)]
        frame: u64,
        #[arg(long)]
        paused: bool,
        #[arg(long)]
        no_loop: bool,
        /// Directory for explicit scene-only PNG captures (S or Save PNG).
        #[arg(long, default_value = "out/preview")]
        capture_dir: PathBuf,
    },
    /// Persistent version 1 JSON-lines host; stdout contains protocol only.
    Serve {
        #[command(flatten)]
        gpu: GpuArgs,
    },
    /// Validate a scene and print resolved values without initializing a GPU.
    Inspect {
        #[command(flatten)]
        scene: SceneArgs,
        #[arg(long, default_value_t = 0)]
        frame: u64,
    },
    /// Save numerical fields and emit a scene that can replay with --require-saved.
    BakeData {
        #[command(flatten)]
        scene: SceneArgs,
        #[arg(long, default_value = "out/baked/scene.json")]
        output: PathBuf,
    },
    /// Write the reference scene as editable versioned JSON.
    DumpScene {
        #[arg(long, default_value = "out/reference.json")]
        output: PathBuf,
    },
}

#[derive(Args)]
struct SceneArgs {
    #[arg(long, default_value = "reference")]
    scene: String,
    #[arg(long)]
    fps: Option<FrameRate>,
    #[arg(long, value_parser = parse_size)]
    size: Option<[u32; 2]>,
    /// Reject any numerical source that requires computation.
    #[arg(long)]
    require_saved: bool,
}
#[derive(Args)]
struct RangeArgs {
    #[arg(long, default_value_t = 0)]
    start: u64,
    /// Exclusive end frame. Defaults to the composition duration at the selected fps.
    #[arg(long)]
    end: Option<u64>,
}
#[derive(Args)]
struct GpuArgs {
    #[arg(long)]
    adapter: Option<String>,
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    #[arg(long)]
    allow_software: bool,
}
impl From<GpuArgs> for GpuOptions {
    fn from(value: GpuArgs) -> Self {
        Self {
            adapter: value.adapter,
            backend: value.backend,
            allow_software: value.allow_software,
        }
    }
}
fn parse_size(text: &str) -> Result<[u32; 2], String> {
    let Some((w, h)) = text.split_once('x') else {
        return Err("expected WIDTHxHEIGHT, e.g. 1280x720".into());
    };
    let size = [
        w.parse::<u32>().map_err(|e| e.to_string())?,
        h.parse::<u32>().map_err(|e| e.to_string())?,
    ];
    if size.iter().any(|v| *v == 0 || *v > 8192) {
        return Err("dimensions must be in 1..=8192".into());
    }
    Ok(size)
}
fn load(args: &SceneArgs) -> Result<CompiledPlan> {
    let (spec, base): (SceneSpec, &Path) = if args.scene == "reference" {
        (reference::scene(), Path::new("."))
    } else {
        let path = Path::new(&args.scene);
        (read_scene(path)?, path.parent().unwrap_or(Path::new(".")))
    };
    let policy = if args.require_saved {
        SourcePolicy::RequireSaved
    } else {
        SourcePolicy::OnDemand
    };
    CompiledPlan::compile_with_policy(spec, base, &mut SourceCache::default(), policy)
}
fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("lemma: {error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Adapters { backend } => {
            let adapters = pollster::block_on(gpu::adapters(&GpuOptions {
                backend,
                ..Default::default()
            }));
            print_json(&adapters)?;
            if adapters.is_empty() {
                bail!("no adapters found; check GPU visibility and the Vulkan loader/ICD");
            }
        }
        Command::RenderFrame {
            scene,
            gpu,
            frame,
            output,
        } => {
            let start = Instant::now();
            let plan = load(&scene)?;
            let packet = plan.evaluate(
                scene.fps.unwrap_or(plan.spec().fps).sample(frame),
                scene.size,
            )?;
            let evaluate_ms = start.elapsed().as_secs_f64() * 1000.;
            let init = Instant::now();
            let mut renderer = pollster::block_on(Renderer::new(&gpu.into()))?;
            let init_ms = init.elapsed().as_secs_f64() * 1000.;
            eprintln!(
                "adapter: {} ({}, {}), driver {}",
                renderer.adapter.name,
                renderer.adapter.backend,
                renderer.adapter.device_type,
                renderer.adapter.driver_info
            );
            let gpu_frame = renderer.render(&packet)?;
            let capture_start = Instant::now();
            let pixels = Capture::default().read_rgba8(&renderer, &gpu_frame)?;
            let readback_ms = capture_start.elapsed().as_secs_f64() * 1000.;
            let encode_start = Instant::now();
            write_png(&output, packet.size, &pixels).context("write rendered frame")?;
            print_json(
                &serde_json::json!({ "output": output, "frame": frame, "seconds": packet.sample.seconds(), "size": packet.size,
                "adapter": renderer.adapter, "timing_ms": {"load_and_evaluate": evaluate_ms, "initialize": init_ms,
                    "readback_including_gpu_wait": readback_ms, "png": encode_start.elapsed().as_secs_f64() * 1000., "total": start.elapsed().as_secs_f64() * 1000.},
                "renderer": renderer.stats }),
            )?;
        }
        Command::RenderRange {
            scene,
            gpu,
            range,
            output,
            overwrite,
        } => {
            let plan = load(&scene)?;
            print_json(&render_range(
                &plan,
                &gpu.into(),
                &RangeOptions {
                    start: range.start,
                    end: range.end,
                    fps: scene.fps,
                    size: scene.size,
                },
                Delivery::PngSequence {
                    directory: &output,
                    overwrite,
                },
            )?)?;
        }
        Command::Export {
            scene,
            gpu,
            range,
            output,
            ffmpeg,
            overwrite,
        } => {
            let plan = load(&scene)?;
            print_json(&render_range(
                &plan,
                &gpu.into(),
                &RangeOptions {
                    start: range.start,
                    end: range.end,
                    fps: scene.fps,
                    size: scene.size,
                },
                Delivery::Mp4 {
                    path: &output,
                    ffmpeg: &ffmpeg,
                    overwrite,
                },
            )?)?;
        }
        Command::Preview {
            scene,
            gpu,
            frame,
            paused,
            no_loop,
            capture_dir,
        } => {
            let plan = load(&scene)?;
            let scene_path = (scene.scene != "reference").then(|| PathBuf::from(&scene.scene));
            let source_policy = if scene.require_saved {
                SourcePolicy::RequireSaved
            } else {
                SourcePolicy::OnDemand
            };
            preview::run(
                plan,
                gpu.into(),
                PreviewOptions {
                    size: scene.size,
                    fps: scene.fps,
                    start_frame: frame,
                    paused,
                    looping: !no_loop,
                    scene_path,
                    source_policy,
                    capture_directory: capture_dir,
                },
            )?;
        }
        Command::Serve { gpu } => {
            lemma::host::serve(io::stdin().lock(), io::stdout().lock(), gpu.into())?;
        }
        Command::Inspect { scene, frame } => {
            let plan = load(&scene)?;
            let packet = plan.evaluate(
                scene.fps.unwrap_or(plan.spec().fps).sample(frame),
                scene.size,
            )?;
            print_json(&plan.inspect(&packet))?;
        }
        Command::BakeData { scene, output } => {
            let plan = load(&scene)?;
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let directory = parent.join("data");
            let mut spec = plan.spec().clone();
            for asset in &mut spec.assets {
                let manifest = plan.fields()[&asset.id].save(&directory)?;
                asset.source = FieldSource::Saved {
                    manifest: Path::new("data").join(manifest.file_name().unwrap()),
                };
            }
            lemma::data::atomic_write(&output, &serde_json::to_vec_pretty(&spec)?)?;
            print_json(
                &serde_json::json!({"output":output,"fields":spec.assets.len(),"policy":"require-saved compatible"}),
            )?;
        }
        Command::DumpScene { output } => {
            lemma::data::atomic_write(&output, &serde_json::to_vec_pretty(&reference::scene())?)?;
            print_json(&serde_json::json!({"output":output}))?;
        }
    }
    Ok(())
}
