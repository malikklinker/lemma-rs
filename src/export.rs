//! Bounded, streaming frame delivery. No complete video or image sequence in RAM.
use crate::{
    eval::CompiledPlan,
    output::AtomicOutput,
    render::{
        Renderer,
        capture::{Capture, write_png_with_overwrite},
        gpu::GpuOptions,
    },
    time::FrameRate,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::Instant,
};

pub struct RangeOptions {
    pub start: u64,
    /// Exclusive end frame; omitted means the composition's duration.
    pub end: Option<u64>,
    pub fps: Option<FrameRate>,
    pub size: Option<[u32; 2]>,
}

#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub output: PathBuf,
    pub frames: u64,
    pub fps: FrameRate,
    pub size: [u32; 2],
    pub adapter: crate::render::gpu::AdapterReport,
    pub renderer: crate::render::RenderStats,
    pub timing_ms: ExportTiming,
}
#[derive(Debug, Default, Serialize)]
pub struct ExportTiming {
    pub initialize: f64,
    pub evaluate: f64,
    /// Total CPU preparation/upload work across the exported range.
    pub prepare_upload_cpu: f64,
    /// Total CPU command recording/submission, not GPU execution duration.
    pub record_submit_cpu: f64,
    pub readback_including_gpu_wait: f64,
    pub encode_or_pipe_wait: f64,
    pub total: f64,
}
pub enum Delivery<'a> {
    PngSequence {
        directory: &'a Path,
        overwrite: bool,
    },
    Mp4 {
        path: &'a Path,
        ffmpeg: &'a Path,
        overwrite: bool,
    },
}

pub fn render_range(
    plan: &CompiledPlan,
    gpu: &GpuOptions,
    range: &RangeOptions,
    delivery: Delivery<'_>,
) -> Result<ExportReport> {
    let start = Instant::now();
    let fps = range.fps.unwrap_or(plan.spec().fps);
    let end = match range.end {
        Some(end) => end,
        None => fps.frame_count(plan.spec().duration)?,
    };
    ensure!(
        end > range.start,
        "exclusive end frame must be greater than start"
    );
    let size = range.size.unwrap_or(plan.spec().size);
    ensure!(
        size.iter().all(|v| *v > 0 && *v <= 8192),
        "export size must be in 1..=8192 pixels per axis"
    );
    // Resolve the first requested sample before starting an encoder or creating
    // output. Compilation alone cannot catch sample-dependent camera/node errors.
    let evaluate = Instant::now();
    let mut first_packet = Some(plan.evaluate(fps.sample(range.start), Some(size))?);
    let first_evaluate_ms = evaluate.elapsed().as_secs_f64() * 1000.;
    let encoding_setup = Instant::now();
    let mut encoder = match &delivery {
        Delivery::Mp4 {
            path,
            ffmpeg,
            overwrite,
        } => Some(VideoEncoder::new(path, ffmpeg, size, fps, *overwrite)?),
        Delivery::PngSequence {
            directory,
            overwrite,
        } => {
            std::fs::create_dir_all(directory)?;
            if !overwrite {
                for frame in range.start..end {
                    ensure!(
                        !directory.join(format!("frame-{frame:06}.png")).exists(),
                        "output frame already exists; pass --overwrite to replace it"
                    );
                }
            }
            None
        }
    };
    let encoding_setup_ms = encoding_setup.elapsed().as_secs_f64() * 1000.;
    let init = Instant::now();
    let mut renderer = pollster::block_on(Renderer::new(gpu))?;
    let mut timing = ExportTiming {
        initialize: init.elapsed().as_secs_f64() * 1000.,
        evaluate: first_evaluate_ms,
        encode_or_pipe_wait: encoding_setup_ms,
        ..Default::default()
    };
    eprintln!(
        "adapter: {} ({}, {}), driver {}",
        renderer.adapter.name,
        renderer.adapter.backend,
        renderer.adapter.device_type,
        renderer.adapter.driver_info
    );
    let mut capture = Capture::default();
    for frame in range.start..end {
        let packet = match first_packet.take() {
            Some(packet) => packet,
            None => {
                let evaluate = Instant::now();
                let packet = plan.evaluate(fps.sample(frame), Some(size))?;
                timing.evaluate += evaluate.elapsed().as_secs_f64() * 1000.;
                packet
            }
        };
        let gpu_frame = renderer.render(&packet)?;
        timing.prepare_upload_cpu += renderer.stats.prepare_ms;
        timing.record_submit_cpu += renderer.stats.submit_ms;
        let readback = Instant::now();
        let pixels = capture.read_rgba8(&renderer, &gpu_frame)?;
        timing.readback_including_gpu_wait += readback.elapsed().as_secs_f64() * 1000.;
        let encode = Instant::now();
        match &delivery {
            Delivery::PngSequence {
                directory,
                overwrite,
            } => write_png_with_overwrite(
                &directory.join(format!("frame-{frame:06}.png")),
                size,
                &pixels,
                *overwrite,
            )?,
            Delivery::Mp4 { .. } => encoder.as_mut().unwrap().write(&pixels)?,
        }
        timing.encode_or_pipe_wait += encode.elapsed().as_secs_f64() * 1000.;
        if (frame - range.start + 1).is_multiple_of(30) || frame + 1 == end {
            eprintln!(
                "rendered {}/{} frames",
                frame - range.start + 1,
                end - range.start
            );
        }
    }
    let encode = Instant::now();
    if let Some(encoder) = encoder {
        encoder.finish()?;
    }
    timing.encode_or_pipe_wait += encode.elapsed().as_secs_f64() * 1000.;
    timing.total = start.elapsed().as_secs_f64() * 1000.;
    let output = match delivery {
        Delivery::PngSequence { directory, .. } => directory.to_owned(),
        Delivery::Mp4 { path, .. } => path.to_owned(),
    };
    Ok(ExportReport {
        output,
        frames: end - range.start,
        fps,
        size,
        adapter: renderer.adapter,
        renderer: renderer.stats,
        timing_ms: timing,
    })
}

struct VideoEncoder {
    child: Child,
    stdin: Option<ChildStdin>,
    output: Option<AtomicOutput>,
    finished: bool,
}
impl VideoEncoder {
    fn new(
        output: &Path,
        ffmpeg: &Path,
        size: [u32; 2],
        fps: FrameRate,
        overwrite: bool,
    ) -> Result<Self> {
        ensure!(
            size.iter().all(|v| *v > 0 && *v <= 8192 && *v % 2 == 0),
            "H.264 yuv420p output requires positive even dimensions <=8192"
        );
        ensure!(
            output
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("mp4")),
            "video output must use the .mp4 extension"
        );
        let output = AtomicOutput::new(output, overwrite)?;
        let mut child = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "rawvideo", "-pixel_format", "rgba", "-video_size"])
            .arg(format!("{}x{}", size[0], size[1])).arg("-framerate").arg(fps.to_string())
            .args(["-i", "pipe:0", "-an", "-vf", "scale=out_color_matrix=bt709:out_range=tv,setparams=range=limited:color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-color_primaries", "bt709", "-color_trc", "iec61966-2-1", "-colorspace", "bt709"])
            .arg(&output.temporary).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn()
            .with_context(|| format!("start FFmpeg at {}; install FFmpeg or pass --ffmpeg PATH", ffmpeg.display()))?;
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            output: Some(output),
            finished: false,
        })
    }
    fn write(&mut self, pixels: &[u8]) -> Result<()> {
        self.stdin
            .as_mut()
            .unwrap()
            .write_all(pixels)
            .context("stream RGBA frame to FFmpeg")
    }
    fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        ensure!(
            status.success(),
            "FFmpeg failed with {status}; incomplete output was not published"
        );
        self.output.take().unwrap().commit()?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for VideoEncoder {
    fn drop(&mut self) {
        if !self.finished {
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn encoder_failure_preserves_existing_output_and_cleans_partial_file() {
        let directory =
            std::env::temp_dir().join(format!("lemma-encoder-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("failing-encoder");
        std::fs::write(&script, "#!/bin/sh\nexit 7\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = directory.join("existing.mp4");
        std::fs::write(&output, b"previous complete video").unwrap();
        let encoder =
            VideoEncoder::new(&output, &script, [128, 72], FrameRate::default(), true).unwrap();
        assert!(
            encoder
                .finish()
                .unwrap_err()
                .to_string()
                .contains("FFmpeg failed")
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"previous complete video");
        let remaining: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            remaining.len(),
            2,
            "partial output must be removed: {remaining:?}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
