//! Native presentation of the same evaluated frames used by PNG capture and export.
//! The presentation pass samples the GPU texture directly; it never reads pixels back.

mod interface;
mod transport;

use interface::{Action, Interface, Layout, TOOLBAR_HEIGHT, UiState};
use transport::Playhead;

use super::{
    Renderer,
    capture::{Capture, write_png_with_overwrite},
    gpu::{self, GpuOptions},
};
use crate::{
    data::{SourceCache, SourcePolicy},
    eval::{CompiledPlan, FramePacket, read_scene},
    time::FrameRate,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorIcon, Window, WindowId},
};

const RELOAD_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    /// Initial scene size; transport controls add space below it. Resizing changes
    /// the evaluated scene dimensions without adding controls to scene content.
    pub size: Option<[u32; 2]>,
    /// Override the scene's rate, including after a reload.
    pub fps: Option<FrameRate>,
    pub start_frame: u64,
    pub paused: bool,
    pub looping: bool,
    /// Watch an authored JSON scene. TSX must first be compiled to that file.
    pub scene_path: Option<PathBuf>,
    /// Preserve numerical execution policy across automatic/manual reloads.
    pub source_policy: SourcePolicy,
    /// Explicit scene-only snapshots; normal preview never reads frames back.
    pub capture_directory: PathBuf,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            size: None,
            fps: None,
            start_frame: 0,
            paused: false,
            looping: true,
            scene_path: None,
            source_policy: SourcePolicy::OnDemand,
            capture_directory: PathBuf::from("out/preview"),
        }
    }
}

/// Open a native window and run until it closes. Must be called on the main thread.
pub fn run(plan: CompiledPlan, gpu_options: GpuOptions, options: PreviewOptions) -> Result<()> {
    let fps = options.fps.unwrap_or(plan.spec().fps);
    let last = last_frame(plan.spec().duration, fps)?;
    ensure!(
        options.start_frame <= last,
        "preview start frame {} is outside the scene's frame range 0..={last}",
        options.start_frame
    );
    let size = options.size.unwrap_or(plan.spec().size);
    let window_size = initial_window_size(size)?;
    plan.evaluate(fps.sample(options.start_frame), Some(size))?;
    if options.source_policy == SourcePolicy::RequireSaved {
        ensure!(
            plan.spec()
                .assets
                .iter()
                .all(|asset| matches!(asset.source, crate::data::FieldSource::Saved { .. })),
            "require-saved preview rejects a plan containing synthetic source requests"
        );
    }
    let source_cache = cache_for_plan(&plan)?;
    let event_loop = EventLoop::new().context(
        "create native event loop; preview requires an accessible X11 or Wayland display",
    )?;
    let instance = gpu::instance(&gpu_options, Some(event_loop.owned_display_handle()));
    let now = Instant::now();
    let stamp = options.scene_path.as_deref().map(file_stamp);
    let mut app = Preview {
        plan,
        source_cache,
        options: options.clone(),
        gpu_options,
        instance,
        clock: Playhead::new(
            fps,
            last,
            options.start_frame,
            !options.paused,
            options.looping,
            now,
        ),
        window: None,
        presentation: None,
        size: window_size,
        cursor: None,
        hover: None,
        capture: Capture::default(),
        capture_requested: false,
        capture_sequence: 0,
        capture_status: None,
        capture_error: false,
        occluded: false,
        dirty: true,
        retry_at: None,
        last_stamp: stamp,
        next_reload: now + RELOAD_INTERVAL,
        reload_error: None,
        fatal_error: None,
    };
    event_loop
        .run_app(&mut app)
        .context("run native preview event loop")?;
    if let Some(error) = app.fatal_error {
        return Err(error);
    }
    Ok(())
}

/// Duration is exclusive, just as for an encoded sequence: an 8 s, 60 fps
/// timeline contains frames 0 through 479.
fn last_frame(duration: f64, fps: FrameRate) -> Result<u64> {
    Ok(fps.frame_count(duration)? - 1)
}

fn initial_window_size(scene: [u32; 2]) -> Result<[u32; 2]> {
    let max_height = 8192 - TOOLBAR_HEIGHT;
    ensure!(
        (1..=8192).contains(&scene[0]) && (1..=max_height).contains(&scene[1]),
        "preview scene width must be in 1..=8192 and height in 1..={max_height} pixels; transport controls need {TOOLBAR_HEIGHT} extra pixels within the 8192-pixel window limit"
    );
    Ok([scene[0], scene[1] + TOOLBAR_HEIGHT])
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}
fn file_stamp(path: &Path) -> Option<FileStamp> {
    std::fs::metadata(path).ok().map(|meta| FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

/// Validate a candidate completely before replacing the last valid scene. Keep
/// equal numerical assets shared across successful camera/palette/text reloads.
fn reload_plan(
    plan: &mut CompiledPlan,
    path: &Path,
    frame: u64,
    size: [u32; 2],
    options: &PreviewOptions,
    source_cache: &mut SourceCache,
    validate_render: impl FnOnce(&FramePacket) -> Result<()>,
) -> Result<(FrameRate, u64)> {
    let candidate = CompiledPlan::compile_with_policy(
        read_scene(path)?,
        path.parent().unwrap_or(Path::new(".")),
        source_cache,
        options.source_policy,
    )?;
    let fps = options.fps.unwrap_or(candidate.spec().fps);
    let last = last_frame(candidate.spec().duration, fps)?;
    let packet = candidate.evaluate(fps.sample(frame.min(last)), Some(size))?;
    validate_render(&packet).context("prepare reloaded scene for GPU rendering")?;
    *plan = candidate;
    Ok((fps, last))
}

fn cache_for_plan(plan: &CompiledPlan) -> Result<SourceCache> {
    let mut cache = SourceCache::default();
    for asset in &plan.spec().assets {
        let field = plan
            .fields()
            .get(&asset.id)
            .with_context(|| format!("compiled plan is missing field {:?}", asset.id))?;
        cache.seed(&asset.source, field.clone())?;
    }
    Ok(cache)
}

struct Presentation {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    adapter: wgpu::Adapter,
    renderer: Renderer,
    interface: Interface,
    configured: bool,
}

struct Preview {
    plan: CompiledPlan,
    source_cache: SourceCache,
    options: PreviewOptions,
    gpu_options: GpuOptions,
    instance: wgpu::Instance,
    clock: Playhead,
    window: Option<Arc<Window>>,
    presentation: Option<Presentation>,
    size: [u32; 2],
    cursor: Option<[f64; 2]>,
    hover: Option<Action>,
    capture: Capture,
    capture_requested: bool,
    capture_sequence: u64,
    capture_status: Option<String>,
    capture_error: bool,
    occluded: bool,
    dirty: bool,
    retry_at: Option<Instant>,
    last_stamp: Option<Option<FileStamp>>,
    next_reload: Instant,
    reload_error: Option<String>,
    fatal_error: Option<anyhow::Error>,
}

impl Preview {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("lemma — initializing selected GPU")
                        .with_inner_size(PhysicalSize::new(self.size[0], self.size[1]))
                        .with_min_inner_size(PhysicalSize::new(320, 180))
                        .with_max_inner_size(PhysicalSize::new(8192, 8192 + TOOLBAR_HEIGHT)),
                )
                .context("create native preview window")?,
        );
        let start = Instant::now();
        let surface = self
            .instance
            .create_surface(window.clone())
            .context("create native GPU surface")?;
        let adapter = pollster::block_on(gpu::select(
            &self.instance,
            &self.gpu_options,
            Some(&surface),
        ))?;
        let renderer = pollster::block_on(Renderer::from_adapter(&adapter))?;
        eprintln!(
            "preview adapter: {} ({}, {}), driver {} ({})",
            renderer.adapter.name,
            renderer.adapter.backend,
            renderer.adapter.device_type,
            renderer.adapter.driver,
            renderer.adapter.driver_info
        );
        let caps = surface.get_capabilities(&adapter);
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("selected adapter cannot configure this window surface")?;
        config.format = caps
            .formats
            .into_iter()
            .find(|format| format.is_srgb())
            .context(
                "window surface exposes no sRGB format; SDR preview requires an sRGB surface",
            )?;
        if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
            config.present_mode = wgpu::PresentMode::Fifo;
        }
        if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        }
        let interface = Interface::new(&renderer.device, &renderer.queue, config.format)?;
        let max = renderer.device.limits().max_texture_dimension_2d;
        window.set_max_inner_size(Some(PhysicalSize::new(
            max.min(8192),
            max.min(8192 + TOOLBAR_HEIGHT),
        )));
        self.size = [size.width, size.height];
        let mut presentation = Presentation {
            surface,
            config,
            adapter,
            renderer,
            interface,
            configured: false,
        };
        if size.width > 0 && size.height > 0 {
            gpu::validate_size(&presentation.renderer.device, self.size)?;
            presentation
                .surface
                .configure(&presentation.renderer.device, &presentation.config);
            presentation.configured = true;
        }
        self.presentation = Some(presentation);
        self.window = Some(window);
        self.clock.seek(self.clock.frame, Instant::now());
        self.dirty = true;
        self.update_title();
        eprintln!(
            "preview initialized in {:.1} ms; Space play/pause, arrows step, Home/End seek, L loop, R reload, S save PNG, Esc exit",
            start.elapsed().as_secs_f64() * 1000.0
        );
        Ok(())
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            let state = if self.clock.playing {
                "playing"
            } else {
                "paused"
            };
            let looping = if self.clock.looping { "loop" } else { "once" };
            let error = if self.reload_error.is_some() {
                " | RELOAD FAILED (last valid scene)"
            } else {
                ""
            };
            window.set_title(&format!("lemma | {} / {} @ {} fps | {state}, {looping} | Space · ←/→ · Home/End · L · R · Esc{error}",
                self.clock.frame, self.clock.last, self.clock.fps));
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<()> {
        self.size = [size.width, size.height];
        self.retry_at = None;
        if let Some(presentation) = &mut self.presentation {
            presentation.configured = false;
            if size.width > 0 && size.height > 0 {
                gpu::validate_size(&presentation.renderer.device, self.size)?;
                presentation.config.width = size.width;
                presentation.config.height = size.height;
                presentation
                    .surface
                    .configure(&presentation.renderer.device, &presentation.config);
                presentation.configured = true;
            }
        }
        self.refresh_hover();
        self.dirty = true;
        Ok(())
    }

    fn reload(&mut self, now: Instant) {
        let Some(path) = self.options.scene_path.as_deref() else {
            return;
        };
        self.clock.advance(now);
        // A minimized window has no renderable size. Validate at its last
        // configured dimensions while preserving its zero-sized surface state.
        let size = if self.size.contains(&0) {
            self.presentation.as_ref().map_or_else(
                || self.options.size.unwrap_or(self.plan.spec().size),
                |presentation| {
                    Layout::new([presentation.config.width, presentation.config.height])
                        .scene_size()
                },
            )
        } else {
            Layout::new(self.size).scene_size()
        };
        match reload_plan(
            &mut self.plan,
            path,
            self.clock.frame,
            size,
            &self.options,
            &mut self.source_cache,
            |packet| match &mut self.presentation {
                Some(presentation) => presentation.renderer.render(packet).map(drop),
                None => Ok(()),
            },
        ) {
            Ok((fps, last)) => {
                self.clock.set_bounds(fps, last, now);
                self.reload_error = None;
                self.dirty = true;
                eprintln!(
                    "preview reloaded {} at frame {}",
                    path.display(),
                    self.clock.frame
                );
            }
            Err(error) => {
                let message = format!("{error:#}");
                if self.reload_error.as_ref() != Some(&message) {
                    eprintln!("preview reload failed; keeping last valid scene: {message}");
                }
                self.reload_error = Some(message);
                self.dirty = true;
            }
        }
        self.update_title();
    }

    fn draw(&mut self) -> Result<()> {
        if self.occluded || self.size.contains(&0) {
            return Ok(());
        }
        if self.retry_at.is_some_and(|retry| Instant::now() < retry) {
            return Ok(());
        }
        let Some(presentation) = &mut self.presentation else {
            return Ok(());
        };
        if !presentation.configured {
            return Ok(());
        }
        let (surface_frame, suboptimal) = match presentation.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.dirty = true;
                self.retry_at = Some(Instant::now() + Duration::from_millis(100));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                presentation
                    .surface
                    .configure(&presentation.renderer.device, &presentation.config);
                self.dirty = true;
                self.retry_at = Some(Instant::now() + Duration::from_millis(16));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                let window = self
                    .window
                    .as_ref()
                    .context("preview surface lost its window")?;
                let surface = self
                    .instance
                    .create_surface(window.clone())
                    .context("recreate lost preview surface")?;
                ensure!(
                    presentation.adapter.is_surface_supported(&surface),
                    "selected adapter no longer supports the recreated preview surface"
                );
                surface.configure(&presentation.renderer.device, &presentation.config);
                presentation.surface = surface;
                self.dirty = true;
                self.retry_at = Some(Instant::now() + Duration::from_millis(16));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                bail!("GPU validation failed while acquiring the preview surface")
            }
        };
        let layout = Layout::new(self.size);
        let packet = self.plan.evaluate(
            self.clock.fps.sample(self.clock.frame),
            Some(layout.scene_size()),
        )?;
        let frame = presentation.renderer.render(&packet)?;
        if std::mem::take(&mut self.capture_requested) {
            let unix_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            self.capture_sequence += 1;
            let output = self.options.capture_directory.join(format!(
                "frame-{:06}-{unix_ms}-{}-{}.png",
                self.clock.frame,
                std::process::id(),
                self.capture_sequence,
            ));
            let result = (|| {
                let pixels = self.capture.read_rgba8(&presentation.renderer, &frame)?;
                write_png_with_overwrite(&output, packet.size, &pixels, false)
            })();
            match result {
                Ok(()) => {
                    let message = format!("Saved {}", output.display());
                    eprintln!("preview capture: {message}");
                    self.capture_status = Some(message);
                    self.capture_error = false;
                }
                Err(error) => {
                    let message = format!("Capture failed: {error:#}");
                    eprintln!("preview {message}");
                    self.capture_status = Some(message);
                    self.capture_error = true;
                }
            }
        }
        presentation.interface.prepare(
            &presentation.renderer.device,
            &presentation.renderer.queue,
            &layout,
            &UiState {
                frame: self.clock.frame,
                last: self.clock.last,
                fps: self.clock.fps,
                playing: self.clock.playing,
                looping: self.clock.looping,
                hover: self.hover,
                scrubbing: self.clock.scrubbing,
                error: self.reload_error.as_deref(),
                reload_available: self.options.scene_path.is_some(),
                adapter_name: &presentation.renderer.adapter.name,
                software: presentation.renderer.adapter.software,
                capture_status: self.capture_status.as_deref(),
                capture_error: self.capture_error,
            },
        )?;
        let target = surface_frame.texture.create_view(&Default::default());
        let mut encoder =
            presentation
                .renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("native preview presentation"),
                });
        presentation.interface.encode(
            &presentation.renderer.device,
            &mut encoder,
            frame.view(),
            &target,
            &layout,
        );
        presentation.renderer.queue.submit([encoder.finish()]);
        if let Some(window) = &self.window {
            window.pre_present_notify();
        }
        presentation.renderer.queue.present(surface_frame);
        if suboptimal {
            presentation
                .surface
                .configure(&presentation.renderer.device, &presentation.config);
        }
        self.dirty = false;
        self.retry_at = None;
        Ok(())
    }

    fn key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        match code {
            KeyCode::Escape => event_loop.exit(),
            KeyCode::Space => self.clock.toggle_play(now),
            KeyCode::ArrowLeft => self.clock.step(false, now),
            KeyCode::ArrowRight => self.clock.step(true, now),
            KeyCode::Home => {
                self.clock.end_scrub();
                self.clock.seek(0, now);
            }
            KeyCode::End => {
                self.clock.end_scrub();
                self.clock.playing = false;
                self.clock.seek(self.clock.last, now);
            }
            KeyCode::KeyL => {
                self.clock.advance(now);
                self.clock.looping = !self.clock.looping;
                self.clock.seek(self.clock.frame, now);
            }
            KeyCode::KeyR => self.reload(now),
            KeyCode::KeyS => self.capture_requested = true,
            _ => return,
        }
        self.dirty = true;
        self.refresh_hover();
        self.update_title();
    }

    fn refresh_hover(&mut self) {
        let hover = self
            .cursor
            .and_then(|position| Layout::new(self.size).hit_test(position))
            .filter(|action| *action != Action::Reload || self.options.scene_path.is_some());
        if self.hover != hover {
            self.hover = hover;
            self.dirty = true;
        }
        if let Some(window) = &self.window {
            window.set_cursor(if self.clock.scrubbing {
                CursorIcon::Grabbing
            } else if self.hover == Some(Action::Scrub) {
                CursorIcon::EwResize
            } else if self.hover.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
        }
    }

    fn pointer_move(&mut self, position: [f64; 2], now: Instant) {
        self.cursor = Some(position);
        if self.clock.scrubbing {
            let frame = Layout::new(self.size).scrub_frame(position[0], self.clock.last);
            if self.clock.scrub_to(frame, now) {
                self.dirty = true;
                self.update_title();
            }
        }
        self.refresh_hover();
    }

    fn pointer_press(&mut self, now: Instant) {
        let Some(action) = self.hover else { return };
        match action {
            Action::TogglePlay => self.clock.toggle_play(now),
            Action::StepBack => self.clock.step(false, now),
            Action::StepForward => self.clock.step(true, now),
            Action::ToggleLoop => {
                self.clock.advance(now);
                self.clock.looping = !self.clock.looping;
                self.clock.seek(self.clock.frame, now);
            }
            Action::Reload => self.reload(now),
            Action::Capture => self.capture_requested = true,
            Action::Scrub => {
                if let Some(position) = self.cursor {
                    let frame = Layout::new(self.size).scrub_frame(position[0], self.clock.last);
                    self.clock.begin_scrub(frame, now);
                }
            }
        }
        self.dirty = true;
        self.refresh_hover();
        self.update_title();
    }
}

impl ApplicationHandler for Preview {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Err(error) = self.resize(size) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                self.dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_move([position.x, position.y], Instant::now());
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.refresh_hover();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.pointer_press(Instant::now());
                } else {
                    self.clock.end_scrub();
                    self.dirty = true;
                    self.refresh_hover();
                }
            }
            WindowEvent::Focused(false) => {
                self.clock.end_scrub();
                self.dirty = true;
                self.refresh_hover();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    // Key-repeat is useful for stepping, but must not repeatedly toggle playback.
                    if !event.repeat || matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
                        self.key(code, event_loop);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if self.clock.advance(Instant::now()) {
                    self.update_title();
                }
                if let Err(error) = self.draw() {
                    self.fail(event_loop, error.context("draw native preview"));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_reload {
            if let Some(path) = self.options.scene_path.as_deref() {
                let stamp = Some(file_stamp(path));
                if stamp != self.last_stamp {
                    self.last_stamp = stamp;
                    self.reload(now);
                }
            }
            self.next_reload = now + RELOAD_INTERVAL;
        }
        if self.clock.advance(now) {
            self.dirty = true;
            self.update_title();
        }
        if self.retry_at.is_some_and(|retry| retry <= now) {
            self.retry_at = None;
        }
        if self.dirty
            && !self.occluded
            && !self.size.contains(&0)
            && self.retry_at.is_none()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        let wake = if self.clock.playing || self.dirty {
            (now + self.clock.redraw_interval()).min(self.next_reload)
        } else {
            self.next_reload
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            self.retry_at.map_or(wake, |retry| wake.min(retry)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_scene_size_reserves_controls_within_the_window_limit() {
        assert_eq!(initial_window_size([640, 360]).unwrap(), [640, 472]);
        assert_eq!(initial_window_size([8192, 8080]).unwrap(), [8192, 8192]);
        for size in [[0, 360], [640, 0], [8193, 360], [640, 8081], [8192, 8192]] {
            let message = initial_window_size(size).unwrap_err().to_string();
            assert!(message.contains("transport controls"), "{message}");
        }
    }

    #[test]
    fn reload_failure_retains_scene_then_recovers_and_reuses_data() {
        let directory =
            std::env::temp_dir().join(format!("lemma-preview-reload-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("scene.json");
        let mut plan = CompiledPlan::compile(crate::reference::scene(), Path::new(".")).unwrap();
        let options = PreviewOptions::default();
        let mut source_cache = cache_for_plan(&plan).unwrap();
        let old_field = plan.fields().values().next().unwrap().clone();
        let old_background = plan.spec().background;
        std::fs::write(&path, "{ broken reload").unwrap();
        assert!(
            reload_plan(
                &mut plan,
                &path,
                90,
                [640, 360],
                &options,
                &mut source_cache,
                |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(plan.spec().background, old_background);
        let mut spec = plan.spec().clone();
        spec.background = [0.2, 0.3, 0.4, 1.0];
        spec.duration = 1.0;
        std::fs::write(&path, serde_json::to_vec(&spec).unwrap()).unwrap();
        assert!(
            reload_plan(
                &mut plan,
                &path,
                90,
                [640, 360],
                &options,
                &mut source_cache,
                |_| bail!("unsupported glyph in candidate renderer preparation")
            )
            .is_err()
        );
        assert_eq!(plan.spec().background, old_background);
        let (fps, last) = reload_plan(
            &mut plan,
            &path,
            90,
            [640, 360],
            &options,
            &mut source_cache,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(plan.spec().background, spec.background);
        assert_eq!(last, 59);
        assert_eq!(fps, FrameRate::default());
        assert!(Arc::ptr_eq(
            plan.fields().values().next().unwrap(),
            &old_field
        ));
        assert_eq!(source_cache.stats().synthetic_computations, 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn require_saved_reload_rejects_synthetic_even_when_cached() {
        let directory =
            std::env::temp_dir().join(format!("lemma-preview-saved-reload-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("scene.json");
        let synthetic = CompiledPlan::compile(crate::reference::scene(), Path::new(".")).unwrap();
        let mut spec = synthetic.spec().clone();
        for asset in &mut spec.assets {
            asset.source = crate::data::FieldSource::Saved {
                manifest: synthetic.fields()[&asset.id].save(&directory).unwrap(),
            };
        }
        let mut plan = CompiledPlan::compile(spec, Path::new(".")).unwrap();
        let mut cache = cache_for_plan(&plan).unwrap();
        let original_hash = plan.fields().values().next().unwrap().content_hash.clone();
        let options = PreviewOptions {
            source_policy: SourcePolicy::RequireSaved,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_vec(&synthetic.spec()).unwrap()).unwrap();
        let error = reload_plan(
            &mut plan,
            &path,
            90,
            [640, 360],
            &options,
            &mut cache,
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("require-saved policy rejects synthetic"));
        assert_eq!(cache.stats().synthetic_computations, 0);
        assert_eq!(
            plan.fields().values().next().unwrap().content_hash,
            original_hash
        );
        assert!(matches!(
            plan.spec().assets[0].source,
            crate::data::FieldSource::Saved { .. }
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
