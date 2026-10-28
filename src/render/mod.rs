//! All wgpu ownership lives here. Rendering and CPU capture are separate operations.
pub mod capture;
pub mod gpu;
mod graphics;
mod paths;
pub mod preview;
mod resources;

use crate::{eval::FramePacket, scene::srgb_to_linear};
use anyhow::Result;
use gpu::{AdapterReport, GpuOptions};
use graphics::{GraphicVertex, Graphics};
use resources::{
    DynamicBuffer, FieldMesh, FieldVertex, Instance, SphereVertex, SurfaceUniform, UniformBinding,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub(crate) struct Targets {
    owner: Arc<()>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    depth_binding: wgpu::BindGroup,
    size: [u32; 2],
}

/// A rendered texture and its submission. Holding it prevents target reuse.
pub struct GpuFrame {
    pub(crate) targets: Arc<Targets>,
    pub(crate) submission: wgpu::SubmissionIndex,
}
impl GpuFrame {
    pub fn size(&self) -> [u32; 2] {
        self.targets.size
    }
    pub fn view(&self) -> &wgpu::TextureView {
        &self.targets.view
    }
    pub fn submission(&self) -> wgpu::SubmissionIndex {
        self.submission.clone()
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RenderStats {
    pub frames: u64,
    pub field_uploads: u64,
    pub field_upload_bytes: u64,
    pub cached_fields: usize,
    pub text_uploads: u64,
    pub cached_textures: usize,
    /// Last frame's CPU preparation/upload work; resource counters are cumulative.
    pub prepare_ms: f64,
    /// Last frame's CPU command recording/submission, not elapsed GPU execution.
    pub submit_ms: f64,
    pub stroke_triangles: usize,
}

pub struct Renderer {
    owner: Arc<()>,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub adapter: AdapterReport,
    samples: u32,
    surface_pipeline: wgpu::RenderPipeline,
    marker_pipeline: wgpu::RenderPipeline,
    path_pipeline: wgpu::RenderPipeline,
    world_graphics_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    surface_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    camera: UniformBinding,
    fields: HashMap<String, FieldMesh>,
    surface_uniforms: HashMap<String, UniformBinding>,
    marker_vertices: wgpu::Buffer,
    marker_indices: wgpu::Buffer,
    marker_count: u32,
    instances: DynamicBuffer,
    paths: DynamicBuffer,
    graphics: Graphics,
    targets: Option<Arc<Targets>>,
    pub stats: RenderStats,
}

impl Renderer {
    pub async fn new(options: &GpuOptions) -> Result<Self> {
        let instance = gpu::instance(options, None);
        let adapter = gpu::select(&instance, options, None).await?;
        Self::from_adapter(&adapter).await
    }

    pub(crate) async fn from_adapter(adapter: &wgpu::Adapter) -> Result<Self> {
        let report = gpu::report(0, adapter);
        let samples = if report.msaa4 { 4 } else { 1 };
        let (device, queue) = gpu::device(adapter).await?;
        let surface_layout = uniform_layout(&device, "surface parameters");
        let camera_layout = uniform_layout(&device, "camera parameters");
        let camera = UniformBinding::new(&device, &camera_layout, 64);
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GPU label visibility"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: samples > 1,
                },
                count: None,
            }],
        });
        let graphics = Graphics::new(&device, &queue)?;
        let surface_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/surface.wgsl"));
        let marker_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/markers.wgsl"));
        let path_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/paths.wgsl"));
        let graphics_source = include_str!("shaders/graphics.wgsl")
            .replace("__DEPTH_DECL__", if samples > 1 { "@group(1) @binding(0) var scene_depth: texture_depth_multisampled_2d;" } else { "@group(1) @binding(0) var scene_depth: texture_depth_2d;" })
            .replace("__DEPTH_READ__", if samples > 1 { "var depth = 1.0; for(var i = 0u; i < textureNumSamples(scene_depth); i += 1u) { depth = min(depth, textureLoad(scene_depth, coord, i)); }" } else { "let depth = textureLoad(scene_depth, coord, 0);" });
        let graphics_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("graphics and GPU occlusion"),
            source: wgpu::ShaderSource::Wgsl(graphics_source.into()),
        });
        let field_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2];
        let sphere_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let instance_attributes = wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4];
        let stroke_attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x2, 3 => Float32x2];
        let graphics_attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
        let field_buffers = [Some(vertex_layout::<FieldVertex>(&field_attributes, false))];
        let marker_buffers = [
            Some(vertex_layout::<SphereVertex>(&sphere_attributes, false)),
            Some(vertex_layout::<Instance>(&instance_attributes, true)),
        ];
        let path_buffers = [Some(vertex_layout::<paths::StrokeVertex>(
            &stroke_attributes,
            false,
        ))];
        let graphics_buffers = [Some(vertex_layout::<GraphicVertex>(
            &graphics_attributes,
            false,
        ))];
        let surface_pipeline = pipeline(
            &device,
            "scalar surface",
            &surface_shader,
            "fs_main",
            &[&surface_layout],
            &field_buffers,
            samples,
            Some(true),
        );
        let marker_pipeline = pipeline(
            &device,
            "instanced markers",
            &marker_shader,
            "fs_main",
            &[&camera_layout],
            &marker_buffers,
            samples,
            Some(true),
        );
        let path_pipeline = pipeline(
            &device,
            "spatial pixel strokes",
            &path_shader,
            "fs_main",
            &[],
            &path_buffers,
            samples,
            Some(false),
        );
        let world_graphics_pipeline = pipeline(
            &device,
            "world graphics plane",
            &graphics_shader,
            "fs_world",
            &[&graphics.layout],
            &graphics_buffers,
            samples,
            Some(false),
        );
        let overlay_pipeline = pipeline(
            &device,
            "screen graphics and projected labels",
            &graphics_shader,
            "fs_overlay",
            &[&graphics.layout, &depth_layout],
            &graphics_buffers,
            1,
            None,
        );
        let (marker_vertices, marker_indices, marker_count) = resources::sphere(&device);
        let instances = DynamicBuffer::new(&device);
        let paths = DynamicBuffer::new(&device);
        Ok(Self {
            owner: Arc::new(()),
            device,
            queue,
            adapter: report,
            samples,
            surface_pipeline,
            marker_pipeline,
            path_pipeline,
            world_graphics_pipeline,
            overlay_pipeline,
            surface_layout,
            depth_layout,
            camera,
            fields: HashMap::new(),
            surface_uniforms: HashMap::new(),
            marker_vertices,
            marker_indices,
            marker_count,
            instances,
            paths,
            graphics,
            targets: None,
            stats: RenderStats::default(),
        })
    }

    fn targets(&mut self, size: [u32; 2]) -> Arc<Targets> {
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.size == size && Arc::strong_count(t) == 1)
        {
            return self.targets.as_ref().unwrap().clone();
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDR output"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let msaa = (self.samples > 1).then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("antialiased world color"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: self.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: COLOR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let depth = self
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shared world depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: self.samples,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let depth_binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shared label visibility"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&depth),
            }],
        });
        let targets = Arc::new(Targets {
            owner: self.owner.clone(),
            texture,
            view,
            msaa,
            depth,
            depth_binding,
            size,
        });
        self.targets = Some(targets.clone());
        targets
    }

    pub fn render(&mut self, packet: &FramePacket) -> Result<GpuFrame> {
        gpu::validate_size(&self.device, packet.size)?;
        let start = Instant::now();
        let targets = self.targets(packet.size);
        let mut field_keys = HashSet::new();
        let mut surface_keys = HashSet::new();
        for draw in &packet.surfaces {
            let hash = &draw.field.content_hash;
            field_keys.insert(hash.clone());
            surface_keys.insert(draw.id.clone());
            if !self.fields.contains_key(hash) {
                let (mesh, bytes) = FieldMesh::upload(&self.device, &draw.field);
                self.fields.insert(hash.clone(), mesh);
                self.stats.field_uploads += 1;
                self.stats.field_upload_bytes += bytes;
            }
            let binding = self
                .surface_uniforms
                .entry(draw.id.clone())
                .or_insert_with(|| {
                    UniformBinding::new(
                        &self.device,
                        &self.surface_layout,
                        std::mem::size_of::<SurfaceUniform>() as u64,
                    )
                });
            let uniforms = SurfaceUniform {
                matrix: packet.view_projection.to_cols_array_2d(),
                model: draw.model.to_cols_array_2d(),
                normal: normal_transform(draw.model).to_cols_array_2d(),
                params: [
                    draw.height,
                    draw.palette.range[0],
                    draw.palette.range[1],
                    draw.shift,
                ],
                palette: draw.palette.stops.map(srgb_to_linear),
                style: [draw.grid, 0., 0., 0.],
            };
            self.queue
                .write_buffer(&binding.buffer, 0, bytemuck::bytes_of(&uniforms));
        }
        self.fields.retain(|k, _| field_keys.contains(k));
        self.surface_uniforms
            .retain(|k, _| surface_keys.contains(k));
        self.queue.write_buffer(
            &self.camera.buffer,
            0,
            bytemuck::cast_slice(&packet.view_projection.to_cols_array()),
        );
        let instances: Vec<_> = packet
            .markers
            .iter()
            .map(|m| Instance {
                position_radius: [m.position[0], m.position[1], m.position[2], m.radius],
                color: srgb_to_linear(m.color),
            })
            .collect();
        self.instances
            .write(&self.device, &self.queue, &instances)?;
        let strokes = paths::tessellate(
            &packet.paths,
            packet.view_projection,
            [packet.viewport[2], packet.viewport[3]],
        );
        self.paths.write(&self.device, &self.queue, &strokes)?;
        let batches = self.graphics.prepare(&self.device, &self.queue, packet)?;
        self.stats.prepare_ms = start.elapsed().as_secs_f64() * 1000.;
        let submit_start = Instant::now();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lemma frame"),
            });
        {
            let c = srgb_to_linear(packet.background);
            let color_attachment = [Some(wgpu::RenderPassColorAttachment {
                view: targets.msaa.as_ref().unwrap_or(&targets.view),
                depth_slice: None,
                resolve_target: targets.msaa.as_ref().map(|_| &targets.view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: c[0] as f64,
                        g: c[1] as f64,
                        b: c[2] as f64,
                        a: c[3] as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world geometry and graphics"),
                color_attachments: &color_attachment,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let [x, y, width, height] = packet.viewport;
            pass.set_viewport(x, y, width, height, 0.0, 1.0);
            let left = x.floor().max(0.0) as u32;
            let top = y.floor().max(0.0) as u32;
            let right = (x + width).ceil().min(packet.size[0] as f32) as u32;
            let bottom = (y + height).ceil().min(packet.size[1] as f32) as u32;
            pass.set_scissor_rect(left, top, (right - left).max(1), (bottom - top).max(1));
            pass.set_pipeline(&self.surface_pipeline);
            for draw in &packet.surfaces {
                let mesh = &self.fields[&draw.field.content_hash];
                pass.set_bind_group(0, &self.surface_uniforms[&draw.id].bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, 0..1);
            }
            if !instances.is_empty() {
                pass.set_pipeline(&self.marker_pipeline);
                pass.set_bind_group(0, &self.camera.bind_group, &[]);
                pass.set_vertex_buffer(0, self.marker_vertices.slice(..));
                pass.set_vertex_buffer(1, self.instances.buffer.slice(..));
                pass.set_index_buffer(self.marker_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.marker_count, 0, 0..instances.len() as u32);
            }
            if !strokes.is_empty() {
                pass.set_pipeline(&self.path_pipeline);
                pass.set_vertex_buffer(0, self.paths.buffer.slice(..));
                pass.draw(0..strokes.len() as u32, 0..1);
            }
            pass.set_pipeline(&self.world_graphics_pipeline);
            pass.set_vertex_buffer(0, self.graphics.buffer.buffer.slice(..));
            for batch in batches.iter().filter(|b| b.world) {
                pass.set_bind_group(0, &self.graphics.textures[&batch.key].bind_group, &[]);
                pass.draw(batch.range.clone(), 0..1);
            }
        }
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &targets.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screen composition"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.overlay_pipeline);
            pass.set_bind_group(1, &targets.depth_binding, &[]);
            pass.set_vertex_buffer(0, self.graphics.buffer.buffer.slice(..));
            for batch in batches.iter().filter(|b| !b.world) {
                pass.set_bind_group(0, &self.graphics.textures[&batch.key].bind_group, &[]);
                pass.draw(batch.range.clone(), 0..1);
            }
        }
        let submission = self.queue.submit([encoder.finish()]);
        self.stats.submit_ms = submit_start.elapsed().as_secs_f64() * 1000.;
        self.stats.frames += 1;
        self.stats.cached_fields = self.fields.len();
        self.stats.cached_textures = self.graphics.textures.len();
        self.stats.text_uploads = self.graphics.uploads;
        self.stats.stroke_triangles = strokes.len() / 3;
        Ok(GpuFrame {
            targets,
            submission,
        })
    }
}

/// Scene models use a positive uniform scale. Normalize their basis before
/// uploading so lighting depends on rotation and remains stable at tiny/large
/// scales, where squaring the raw scaled normals in a shader can under/overflow.
fn normal_transform(model: glam::Mat4) -> glam::Mat4 {
    let axis = |axis: glam::Vec4| {
        let axis = axis.truncate();
        (axis / axis.abs().max_element()).normalize().extend(0.0)
    };
    glam::Mat4::from_cols(
        axis(model.x_axis),
        axis(model.y_axis),
        axis(model.z_axis),
        glam::Vec4::W,
    )
}

fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}
fn vertex_layout<'a, T>(
    attributes: &'a [wgpu::VertexAttribute],
    instanced: bool,
) -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<T>() as u64,
        step_mode: if instanced {
            wgpu::VertexStepMode::Instance
        } else {
            wgpu::VertexStepMode::Vertex
        },
        attributes,
    }
}
#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    fragment: &str,
    layouts: &[&wgpu::BindGroupLayout],
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    samples: u32,
    depth: Option<bool>,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts.iter().map(|x| Some(*x)).collect::<Vec<_>>(),
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: depth.map(|write| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::normal_transform;
    use glam::{EulerRot, Mat4, Quat, Vec3};

    #[test]
    fn surface_normals_follow_rotation_independent_of_uniform_scale() {
        let rotation = Quat::from_euler(EulerRot::XYZ, 0.2, -0.5, 0.7);
        let expected = Mat4::from_quat(rotation);
        for scale in [1e-30, 0.5, 1.0, 2.0, 1e30] {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(scale),
                rotation,
                Vec3::new(1.0, 2.0, 3.0),
            );
            let actual = normal_transform(model);
            for (a, b) in actual
                .to_cols_array()
                .into_iter()
                .zip(expected.to_cols_array())
            {
                assert!((a - b).abs() < 1e-6);
            }
        }
    }
}
