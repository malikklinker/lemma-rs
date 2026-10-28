//! Native preview controls. These GPU graphics are separate from the authored
//! scene, so captures/exports and numerical resource identities stay unchanged.
use crate::{
    eval::{GraphicDraw, Placement},
    render::graphics::{GraphicBatch, GraphicRect, GraphicVertex, Graphics},
    scene::Color,
    time::FrameRate,
};
use anyhow::Result;

pub(super) const TOOLBAR_HEIGHT: u32 = 112;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    TogglePlay,
    StepBack,
    StepForward,
    ToggleLoop,
    Reload,
    Capture,
    Scrub,
}

#[derive(Debug, Clone, Copy)]
struct Rect([f32; 4]);
impl Rect {
    fn contains(self, point: [f64; 2]) -> bool {
        let [x, y, w, h] = self.0.map(f64::from);
        point[0] >= x && point[0] < x + w && point[1] >= y && point[1] < y + h
    }
}

#[derive(Debug, Clone)]
pub(super) struct Layout {
    size: [u32; 2],
    scene: [u32; 2],
    buttons: Vec<(Action, Rect)>,
    timeline: Rect,
}

impl Layout {
    pub(super) fn new(size: [u32; 2]) -> Self {
        let scene = [
            size[0].max(1),
            size[1].saturating_sub(TOOLBAR_HEIGHT).max(1),
        ];
        let top = scene[1] as f32;
        let width = size[0] as f32;
        let compact = width < 640.0;
        let factor = (width / 320.0).min(1.0);
        let make = |x, w| Rect([x * factor, top + 12.0, w * factor, 32.0]);
        let mut buttons = if compact {
            vec![
                (Action::TogglePlay, make(16.0, 64.0)),
                (Action::StepBack, make(88.0, 28.0)),
                (Action::StepForward, make(124.0, 28.0)),
                (Action::ToggleLoop, make(160.0, 52.0)),
                (
                    Action::Reload,
                    Rect([
                        (width - 80.0 * factor).max(0.0),
                        top + 12.0,
                        64.0 * factor,
                        32.0,
                    ]),
                ),
            ]
        } else {
            vec![
                (Action::TogglePlay, make(16.0, 88.0)),
                (Action::StepBack, make(112.0, 32.0)),
                (Action::StepForward, make(152.0, 32.0)),
                (Action::ToggleLoop, make(192.0, 64.0)),
                (Action::Reload, Rect([width - 96.0, top + 12.0, 80.0, 32.0])),
            ]
        };
        if width >= 800.0 {
            buttons.push((
                Action::Capture,
                Rect([width - 188.0, top + 12.0, 84.0, 32.0]),
            ));
        }
        Self {
            size,
            scene,
            buttons,
            timeline: Rect([
                20.0 * factor,
                top + 52.0,
                (width - 40.0 * factor).max(1.0),
                24.0,
            ]),
        }
    }

    pub(super) fn scene_size(&self) -> [u32; 2] {
        self.scene
    }

    pub(super) fn hit_test(&self, point: [f64; 2]) -> Option<Action> {
        if point.iter().any(|v| !v.is_finite()) {
            return None;
        }
        self.buttons
            .iter()
            .find_map(|(action, rect)| rect.contains(point).then_some(*action))
            .or_else(|| {
                let [x, y, width, height] = self.timeline.0;
                // Include both endpoint thumbs, whose centers sit on the track
                // boundaries and whose radius extends beyond the filled rail.
                Rect([x - 6.0, y, width + 12.0, height])
                    .contains(point)
                    .then_some(Action::Scrub)
            })
    }

    pub(super) fn scrub_frame(&self, x: f64, last: u64) -> u64 {
        let [left, _, width, _] = self.timeline.0.map(f64::from);
        let fraction = ((x - left) / width).clamp(0.0, 1.0);
        (fraction * last as f64).round() as u64
    }
}

pub(super) struct UiState<'a> {
    pub frame: u64,
    pub last: u64,
    pub fps: FrameRate,
    pub playing: bool,
    pub looping: bool,
    pub hover: Option<Action>,
    pub scrubbing: bool,
    pub error: Option<&'a str>,
    pub reload_available: bool,
    pub adapter_name: &'a str,
    pub software: bool,
    pub capture_status: Option<&'a str>,
    pub capture_error: bool,
}

pub(super) struct Interface {
    graphics: Graphics,
    batches: Vec<GraphicBatch>,
    pipeline: wgpu::RenderPipeline,
    scene_pipeline: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Interface {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        let graphics = Graphics::new(device, queue)?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("native preview interface"),
            source: wgpu::ShaderSource::Wgsl(include_str!("interface.wgsl").into()),
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview scene texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("preview scene sampling"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
        let vertices = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GraphicVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let make_pipeline = |name, vertex, fragment, layout: &wgpu::BindGroupLayout, buffers| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(name),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline(
            "preview chrome",
            "vs_ui",
            "fs_ui",
            &graphics.layout,
            &vertices,
        );
        let scene_pipeline =
            make_pipeline("preview scene", "vs_scene", "fs_scene", &scene_layout, &[]);
        Ok(Self {
            graphics,
            batches: Vec::new(),
            pipeline,
            scene_pipeline,
            scene_layout,
            sampler,
        })
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &Layout,
        state: &UiState<'_>,
    ) -> Result<()> {
        let mut rectangles = Vec::new();
        let mut text = Vec::new();
        let top = layout.scene[1] as f32;
        let width = layout.size[0] as f32;
        let panel = rgb(0x101820);
        let border = rgb(0x29353f);
        let muted = rgb(0x81929f);
        let bright = rgb(0xe6edf1);
        let accent = rgb(0x62d4c5);
        let mut rect = |bounds, color, radius| {
            rectangles.push(GraphicRect {
                rect: bounds,
                color,
                radius,
            })
        };
        rect([0.0, top, width, TOOLBAR_HEIGHT as f32], panel, 0.0);
        rect([0.0, top, width, 1.0], border, 0.0);
        for (action, bounds) in &layout.buttons {
            let disabled = *action == Action::Reload && !state.reload_available;
            let selected =
                *action == Action::TogglePlay || (*action == Action::ToggleLoop && state.looping);
            let hovered = state.hover == Some(*action) && !disabled;
            let background = if selected {
                rgb(0x244440)
            } else if hovered {
                rgb(0x2a3844)
            } else {
                rgb(0x1a252f)
            };
            rect(bounds.0, background, 6.0);
            let label = match action {
                Action::TogglePlay if state.playing => "PAUSE",
                Action::TogglePlay => "PLAY",
                Action::StepBack => "<",
                Action::StepForward => ">",
                Action::ToggleLoop => "LOOP",
                Action::Reload => "RELOAD",
                Action::Capture => "SAVE PNG",
                Action::Scrub => unreachable!(),
            };
            let font_size = if matches!(action, Action::StepBack | Action::StepForward) {
                17.0
            } else {
                11.0
            };
            let extent = crate::text::extent(label, font_size)?;
            let x = bounds.0[0] + (bounds.0[2] - extent[0] as f32) * 0.5;
            let y = bounds.0[1] + (bounds.0[3] - extent[1] as f32) * 0.5;
            add_text(
                &mut text,
                label.into(),
                [x, y],
                font_size,
                if disabled {
                    rgb(0x4d5c68)
                } else if selected {
                    accent
                } else {
                    bright
                },
            );
        }
        let seconds = state.fps.sample(state.frame).seconds();
        let info = format!(
            "FRAME {:04} / {:04}    {:.3} s",
            state.frame, state.last, seconds
        );
        if width >= 640.0 {
            let right = if width >= 800.0 {
                width - 200.0
            } else {
                width - 108.0
            };
            add_text(
                &mut text,
                fit_label(&info, (right - 276.0).max(0.0), 12.0)?,
                [276.0, top + 21.0],
                12.0,
                bright,
            );
        }
        let [left, y, rail_width, _] = layout.timeline.0;
        let progress = if state.last == 0 {
            0.0
        } else {
            (state.frame as f64 / state.last as f64).clamp(0.0, 1.0) as f32
        };
        rect([left, y + 10.0, rail_width, 4.0], border, 2.0);
        rect([left, y + 10.0, rail_width * progress, 4.0], accent, 2.0);
        let knob = if state.scrubbing || state.hover == Some(Action::Scrub) {
            12.0
        } else {
            10.0
        };
        rect(
            [
                left + rail_width * progress - knob * 0.5,
                y + 12.0 - knob * 0.5,
                knob,
                knob,
            ],
            bright,
            knob * 0.5,
        );
        let footer_y = top + 84.0;
        let message = match state.error {
            Some(error) => format!(
                "RELOAD FAILED  {}",
                fit_ascii(error, ((width - 40.0).max(0.0) / 6.5) as usize)
            ),
            None if state.capture_status.is_some() => state.capture_status.unwrap().into(),
            None if width < 640.0 => info,
            None if state.scrubbing => "SCRUBBING  Release to keep this frame".into(),
            None if state.reload_available => "LIVE SCENE  Edits reload automatically".into(),
            None => "REFERENCE SCENE  Space play/pause  |  Arrows step  |  S save PNG".into(),
        };
        let footer_width = if state.error.is_some() || width < 640.0 {
            width - 40.0
        } else if width < 900.0 {
            width - 175.0
        } else {
            width - 290.0
        };
        add_text(
            &mut text,
            fit_label(&message, footer_width.max(0.0), 11.0)?,
            [20.0, footer_y],
            11.0,
            if state.error.is_some() || (state.capture_status.is_some() && state.capture_error) {
                rgb(0xf3ab86)
            } else {
                muted
            },
        );
        if state.error.is_none() && width >= 900.0 {
            let badge = if state.software {
                "SOFTWARE GPU"
            } else {
                "HARDWARE GPU"
            };
            let name = fit_ascii(state.adapter_name, 22);
            let label = fit_label(&format!("{badge}  {name}"), 250.0, 10.0)?;
            let extent = crate::text::extent(&label, 10.0)?;
            add_text(
                &mut text,
                label,
                [width - extent[0] as f32 - 20.0, footer_y + 1.0],
                10.0,
                if state.software { rgb(0xc5aa7a) } else { muted },
            );
        } else if state.error.is_none() && width >= 640.0 {
            let label = if state.software {
                "SOFTWARE GPU"
            } else {
                "GPU PREVIEW"
            };
            let extent = crate::text::extent(label, 10.0)?;
            add_text(
                &mut text,
                label.into(),
                [width - extent[0] as f32 - 20.0, footer_y + 1.0],
                10.0,
                if state.software { rgb(0xc5aa7a) } else { muted },
            );
        }
        self.batches = self
            .graphics
            .prepare_ui(device, queue, layout.size, &rectangles, &text)?;
        Ok(())
    }

    pub(super) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView,
        target: &wgpu::TextureView,
        layout: &Layout,
    ) {
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("native scene composition"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("native scene and playback interface"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            0.0,
            0.0,
            layout.scene[0] as f32,
            layout.scene[1] as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.scene_pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.draw(0..3, 0..1);
        pass.set_viewport(
            0.0,
            0.0,
            layout.size[0] as f32,
            layout.size[1] as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.graphics.buffer.buffer.slice(..));
        for batch in &self.batches {
            pass.set_bind_group(0, &self.graphics.textures[&batch.key].bind_group, &[]);
            pass.draw(batch.range.clone(), 0..1);
        }
    }
}

fn add_text(
    text: &mut Vec<GraphicDraw>,
    value: String,
    position: [f32; 2],
    font_size: f32,
    color: Color,
) {
    if value.is_empty() {
        return;
    }
    text.push(GraphicDraw {
        id: "preview-interface".into(),
        text: value,
        font_size,
        color,
        placement: Placement::Screen { position },
    });
}

fn rgb(value: u32) -> Color {
    [
        ((value >> 16) & 255) as f32 / 255.0,
        ((value >> 8) & 255) as f32 / 255.0,
        (value & 255) as f32 / 255.0,
        1.0,
    ]
}

fn fit_ascii(value: &str, count: usize) -> String {
    let mut text: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .take(count)
        .collect();
    if value.chars().count() > count && count >= 3 {
        text.truncate(count - 3);
        text.push_str("...");
    }
    text
}

fn fit_label(value: &str, width: f32, font_size: f32) -> Result<String> {
    if width <= 0.0 {
        return Ok(String::new());
    }
    // Bound work before measuring even if an upstream diagnostic is enormous.
    let text = fit_ascii(
        value,
        ((width / (font_size * 0.2)).ceil() as usize + 1).min(512),
    );
    if crate::text::extent(&text, font_size)?[0] as f32 <= width {
        return Ok(text);
    }
    if crate::text::extent("...", font_size)?[0] as f32 > width {
        return Ok(String::new());
    }
    let (mut low, mut high) = (0, text.len());
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let candidate = format!("{}...", &text[..middle]);
        if crate::text::extent(&candidate, font_size)?[0] as f32 <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Ok(format!("{}...", &text[..low]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_and_controls_stay_separate_after_resize() {
        for size in [[640, 472], [800, 562], [320, 300]] {
            let layout = Layout::new(size);
            assert_eq!(layout.scene_size(), [size[0], size[1] - TOOLBAR_HEIGHT]);
            assert_eq!(layout.hit_test([size[0] as f64 * 0.5, 10.0]), None);
            for &(action, Rect([x, y, w, h])) in &layout.buttons {
                assert_eq!(
                    layout.hit_test([(x + w * 0.5) as f64, (y + h * 0.5) as f64]),
                    Some(action)
                );
                assert!(x >= 0.0 && x + w <= size[0] as f32);
            }
        }
    }

    #[test]
    fn timeline_maps_dragged_endpoints_and_exact_midpoints() {
        let layout = Layout::new([800, 562]);
        assert_eq!(layout.scrub_frame(-100.0, 479), 0);
        assert_eq!(layout.scrub_frame(900.0, 479), 479);
        assert_eq!(layout.scrub_frame(400.0, 480), 240);
        assert_eq!(layout.scrub_frame(400.0, 0), 0);
        assert_eq!(layout.hit_test([400.0, 514.0]), Some(Action::Scrub));
        assert_eq!(layout.hit_test([780.0, 514.0]), Some(Action::Scrub));
        assert_eq!(layout.scrub_frame(780.0, 479), 479);
        assert_eq!(layout.hit_test([15.0, 514.0]), Some(Action::Scrub));
        assert_eq!(layout.hit_test([f64::NAN, 500.0]), None);
    }

    #[test]
    fn diagnostics_are_single_line_supported_text_and_bounded() {
        assert_eq!(fit_ascii("bad\nvalue 🦀", 40), "bad value  ");
        assert_eq!(fit_ascii("long diagnostic", 7), "long...");
        assert_eq!(fit_ascii("abcd", 0), "");
        let label = fit_label("WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW", 85.0, 11.0).unwrap();
        assert!(crate::text::extent(&label, 11.0).unwrap()[0] <= 85);
        assert!(label.ends_with("..."));
    }
}
