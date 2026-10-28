use super::resources::DynamicBuffer;
use crate::{
    eval::{GraphicDraw, Placement},
    scene::{Color, Occlusion, srgb_to_linear},
};
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::collections::{HashMap, HashSet};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GraphicVertex {
    pub clip: [f32; 4],
    pub uv: [f32; 2],
    pub color: Color,
    pub anchor: [f32; 4],
}
pub struct TextTexture {
    pub bind_group: wgpu::BindGroup,
    pub size: [u32; 2],
}
pub struct GraphicBatch {
    pub key: String,
    pub range: std::ops::Range<u32>,
    pub world: bool,
}
/// Preview-only panel geometry. Scene authoring does not expose these controls.
pub(super) struct GraphicRect {
    pub rect: [f32; 4],
    pub color: Color,
    pub radius: f32,
}
pub struct Graphics {
    pub textures: HashMap<String, TextTexture>,
    pub buffer: DynamicBuffer,
    pub layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pub uploads: u64,
}

impl Graphics {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cached graphics texture"),
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
            label: Some("graphics linear sampling"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut result = Self {
            textures: HashMap::new(),
            buffer: DynamicBuffer::new(device),
            layout,
            sampler,
            uploads: 0,
        };
        result.upload(device, queue, "white".into(), [1, 1], &[255]);
        Ok(result)
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: String,
        size: [u32; 2],
        pixels: &[u8],
    ) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cached glyph coverage"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0]),
                rows_per_image: Some(size[1]),
            },
            texture.size(),
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cached glyph coverage"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.textures.insert(key, TextTexture { bind_group, size });
        self.uploads += 1;
    }

    fn text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        font_size: f32,
    ) -> Result<String> {
        let key = format!("{}:{:08x}", text, font_size.to_bits());
        if self.textures.contains_key(&key) {
            return Ok(key);
        }
        let (glyphs, [width, height]) = crate::text::layout(text, font_size)?;
        let limit = device.limits().max_texture_dimension_2d;
        ensure!(
            width <= limit && height <= limit && u64::from(width) * u64::from(height) <= 16_777_216,
            "text texture {width}x{height} is too large; split the text into smaller runs"
        );
        let mut pixels = vec![0; (width * height) as usize];
        for glyph in &glyphs {
            let (_, bitmap) = crate::text::font().rasterize_config(glyph.key);
            for y in 0..glyph.height {
                for x in 0..glyph.width {
                    let (px, py) = (glyph.x as i32 + x as i32, glyph.y as i32 + y as i32);
                    if px >= 0 && py >= 0 && px < width as i32 && py < height as i32 {
                        let dest = (py as u32 * width + px as u32) as usize;
                        pixels[dest] = pixels[dest].max(bitmap[y * glyph.width + x]);
                    }
                }
            }
        }
        self.upload(device, queue, key.clone(), [width, height], &pixels);
        Ok(key)
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        packet: &crate::eval::FramePacket,
    ) -> Result<Vec<GraphicBatch>> {
        let mut vertices = Vec::new();
        let mut batches = Vec::new();
        let mut used = HashSet::from(["white".to_owned()]);
        for draw in &packet.graphics {
            let key = self
                .text(device, queue, &draw.text, draw.font_size)
                .with_context(|| format!("node {:?}", draw.id))?;
            used.insert(key.clone());
            let size = self.textures[&key].size;
            if let Some(quad) = graphic_quad(
                draw,
                size,
                packet.view_projection,
                packet.size,
                packet.viewport,
            ) {
                let start = vertices.len() as u32;
                for i in [0, 1, 2, 0, 2, 3] {
                    vertices.push(quad[i]);
                }
                batches.push(GraphicBatch {
                    key,
                    range: start..vertices.len() as u32,
                    world: matches!(draw.placement, Placement::World { .. }),
                });
            }
        }
        for legend in &packet.legends {
            let start = vertices.len() as u32;
            let stops = legend_stops(legend.shift);
            let colors = legend.palette.stops.map(srgb_to_linear);
            for interval in stops.windows(2) {
                let (a, b) = (interval[0], interval[1]);
                let positions = [
                    [legend.position[0] + a * legend.size[0], legend.position[1]],
                    [legend.position[0] + b * legend.size[0], legend.position[1]],
                    [
                        legend.position[0] + b * legend.size[0],
                        legend.position[1] + legend.size[1],
                    ],
                    [
                        legend.position[0] + a * legend.size[0],
                        legend.position[1] + legend.size[1],
                    ],
                ];
                let color = |t: f32| palette_color(&colors, t + legend.shift);
                let quad: [_; 4] = std::array::from_fn(|j| GraphicVertex {
                    clip: screen_clip(positions[j], packet.size),
                    uv: [0.5, 0.5],
                    color: color(if j == 0 || j == 3 { a } else { b }),
                    anchor: [0.; 4],
                });
                for j in [0, 1, 2, 0, 2, 3] {
                    vertices.push(quad[j]);
                }
            }
            batches.push(GraphicBatch {
                key: "white".into(),
                range: start..vertices.len() as u32,
                world: false,
            });
        }
        self.textures.retain(|key, _| used.contains(key));
        self.buffer.write(device, queue, &vertices)?;
        Ok(batches)
    }

    /// Build native preview chrome with its own text cache. The interface shader
    /// interprets rectangle anchor values as rounded coverage parameters.
    pub(super) fn prepare_ui(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        rectangles: &[GraphicRect],
        text: &[GraphicDraw],
    ) -> Result<Vec<GraphicBatch>> {
        let mut vertices = Vec::new();
        let mut batches = Vec::new();
        let mut used = HashSet::from(["white".to_owned()]);
        for rectangle in rectangles {
            let [x, y, width, height] = rectangle.rect;
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            let positions = [
                [x, y],
                [x + width, y],
                [x + width, y + height],
                [x, y + height],
            ];
            let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            let quad: [_; 4] = std::array::from_fn(|i| GraphicVertex {
                clip: screen_clip(positions[i], size),
                uv: uv[i],
                color: srgb_to_linear(rectangle.color),
                anchor: [width, height, rectangle.radius, 1.0],
            });
            for i in [0, 1, 2, 0, 2, 3] {
                vertices.push(quad[i]);
            }
        }
        if !vertices.is_empty() {
            batches.push(GraphicBatch {
                key: "white".into(),
                range: 0..vertices.len() as u32,
                world: false,
            });
        }
        for draw in text {
            let key = self.text(device, queue, &draw.text, draw.font_size)?;
            used.insert(key.clone());
            let texture_size = self.textures[&key].size;
            if let Some(quad) = graphic_quad(
                draw,
                texture_size,
                Mat4::IDENTITY,
                size,
                [0.0, 0.0, size[0] as f32, size[1] as f32],
            ) {
                let start = vertices.len() as u32;
                for i in [0, 1, 2, 0, 2, 3] {
                    vertices.push(quad[i]);
                }
                batches.push(GraphicBatch {
                    key,
                    range: start..vertices.len() as u32,
                    world: false,
                });
            }
        }
        self.textures.retain(|key, _| used.contains(key));
        self.buffer.write(device, queue, &vertices)?;
        Ok(batches)
    }
}

/// The palette is piecewise linear. Splitting only at its shifted stops gives
/// the exact gradient, including clamped ends, without fixed subdivisions.
fn legend_stops(shift: f32) -> Vec<f32> {
    let mut stops = vec![0.0, 1.0];
    stops.extend((0..=4).map(|i| (i as f32 / 4.0 - shift).clamp(0.0, 1.0)));
    stops.sort_by(f32::total_cmp);
    stops.dedup();
    stops
}

fn palette_color(colors: &[Color; 5], value: f32) -> Color {
    let t = value.clamp(0.0, 1.0) * 4.0;
    let i = (t as usize).min(3);
    std::array::from_fn(|n| colors[i][n] + (colors[i + 1][n] - colors[i][n]) * (t - i as f32))
}

fn screen_clip(p: [f32; 2], size: [u32; 2]) -> [f32; 4] {
    [
        p[0] / size[0] as f32 * 2.0 - 1.0,
        1.0 - p[1] / size[1] as f32 * 2.0,
        0.0,
        1.0,
    ]
}

fn graphic_quad(
    draw: &GraphicDraw,
    texture_size: [u32; 2],
    matrix: Mat4,
    viewport: [u32; 2],
    world_viewport: [f32; 4],
) -> Option<[GraphicVertex; 4]> {
    let mut anchor = [0.; 4];
    let [width, height] = texture_size.map(|x| x as f32);
    let clips: [[f32; 4]; 4] = match draw.placement {
        Placement::World {
            position,
            right,
            up,
            width: world_width,
        } => {
            let p = Vec3::from(position);
            let r = Vec3::from(right).normalize() * world_width;
            let down = -Vec3::from(up).normalize() * world_width * height / width;
            [p, p + r, p + r + down, p + down].map(|p| (matrix * p.extend(1.)).to_array())
        }
        _ => {
            let position = match draw.placement {
                Placement::Screen { position } => position,
                Placement::Projected {
                    anchor: point,
                    offset,
                    occlusion,
                } => {
                    let clip = matrix * Vec3::from(point).extend(1.);
                    if clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
                        return None;
                    }
                    let ndc = clip.truncate() / clip.w;
                    let px = world_viewport[0] + (ndc.x * 0.5 + 0.5) * world_viewport[2];
                    let py = world_viewport[1] + (0.5 - ndc.y * 0.5) * world_viewport[3];
                    anchor = [
                        px,
                        py,
                        ndc.z,
                        if occlusion == Occlusion::Hide {
                            1.0
                        } else {
                            0.0
                        },
                    ];
                    [px + offset[0], py + offset[1]]
                }
                Placement::World { .. } => unreachable!(),
            };
            [
                [position[0], position[1]],
                [position[0] + width, position[1]],
                [position[0] + width, position[1] + height],
                [position[0], position[1] + height],
            ]
            .map(|p| screen_clip(p, viewport))
        }
    };
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
    Some(std::array::from_fn(|i| GraphicVertex {
        clip: clips[i],
        uv: uv[i],
        color: srgb_to_linear(draw.color),
        anchor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Palette;

    #[test]
    fn legend_intervals_match_the_full_palette_in_linear_light() {
        let colors = Palette::default().stops.map(srgb_to_linear);
        for shift in [-2.0, -0.31, -0.1, 0.0, 0.1, 0.31, 2.0] {
            let stops = legend_stops(shift);
            assert!(stops.len() <= 6, "at most five gradient strips are needed");
            assert_eq!(stops.first(), Some(&0.0));
            assert_eq!(stops.last(), Some(&1.0));
            for interval in stops.windows(2) {
                let [a, b] = [interval[0], interval[1]];
                let ca = palette_color(&colors, a + shift);
                let cb = palette_color(&colors, b + shift);
                for fraction in [0.0, 0.1, 0.5, 0.9, 1.0] {
                    let actual = palette_color(&colors, a + (b - a) * fraction + shift);
                    for channel in 0..4 {
                        let interpolated = ca[channel] + (cb[channel] - ca[channel]) * fraction;
                        assert!((actual[channel] - interpolated).abs() < 1e-6);
                    }
                }
            }
        }
    }
}
