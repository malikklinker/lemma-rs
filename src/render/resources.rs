use crate::{data::ScalarField, scene::Color};
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FieldVertex {
    position: [f32; 3],
    gradient: [f32; 2],
    grid: [f32; 2],
}
pub struct FieldMesh {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub count: u32,
}

impl FieldMesh {
    pub fn upload(device: &wgpu::Device, field: &ScalarField) -> (Self, u64) {
        let mut vertices = Vec::with_capacity(field.values.len());
        let dx = (field.bounds[1] - field.bounds[0]) / (field.width - 1) as f32;
        let dz = (field.bounds[3] - field.bounds[2]) / (field.height - 1) as f32;
        let value = |x: u32, z: u32| field.values[(z * field.width + x) as usize];
        for z in 0..field.height {
            for x in 0..field.width {
                let [px, pz] = field.position(x, z);
                let (left, right, top, bottom) = (
                    x.saturating_sub(1),
                    (x + 1).min(field.width - 1),
                    z.saturating_sub(1),
                    (z + 1).min(field.height - 1),
                );
                vertices.push(FieldVertex {
                    position: [px, value(x, z), pz],
                    gradient: [
                        ((f64::from(value(right, z)) - f64::from(value(left, z)))
                            / (f64::from(right - left) * f64::from(dx)))
                        .clamp(-1e18, 1e18) as f32,
                        ((f64::from(value(x, bottom)) - f64::from(value(x, top)))
                            / (f64::from(bottom - top) * f64::from(dz)))
                        .clamp(-1e18, 1e18) as f32,
                    ],
                    grid: [x as f32, z as f32],
                });
            }
        }
        let mut indices =
            Vec::<u32>::with_capacity(((field.width - 1) * (field.height - 1) * 6) as usize);
        for z in 0..field.height - 1 {
            for x in 0..field.width - 1 {
                let a = z * field.width + x;
                let b = a + 1;
                let c = a + field.width;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let bytes = (std::mem::size_of_val(vertices.as_slice())
            + std::mem::size_of_val(indices.as_slice())) as u64;
        (
            Self {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("immutable field vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("immutable grid topology"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                count: indices.len() as u32,
            },
            bytes,
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SurfaceUniform {
    pub matrix: [[f32; 4]; 4],
    pub model: [[f32; 4]; 4],
    pub normal: [[f32; 4]; 4],
    pub params: [f32; 4],
    pub palette: [Color; 5],
    pub style: [f32; 4],
}
pub struct UniformBinding {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}
impl UniformBinding {
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: u64) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw parameters"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draw parameters"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self { buffer, bind_group }
    }
}

pub struct DynamicBuffer {
    pub buffer: wgpu::Buffer,
    capacity: u64,
}
impl DynamicBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("reusable frame vertices"),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: 4,
        }
    }
    pub fn write<T: Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[T],
    ) -> Result<()> {
        let bytes = bytemuck::cast_slice(data);
        if bytes.is_empty() {
            return Ok(());
        }
        ensure!(
            bytes.len() as u64 <= device.limits().max_buffer_size,
            "frame vertex data needs {} bytes, exceeding the device buffer limit {}; reduce sample count or split the drawing",
            bytes.len(),
            device.limits().max_buffer_size
        );
        if bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64)
                .next_power_of_two()
                .min(device.limits().max_buffer_size);
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("reusable frame vertices"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytes);
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SphereVertex {
    position: [f32; 3],
    normal: [f32; 3],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Instance {
    pub position_radius: [f32; 4],
    pub color: Color,
}

pub fn sphere(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let (slices, stacks) = (24, 16);
    let mut vertices = Vec::new();
    let mut indices = Vec::<u32>::new();
    for row in 0..=stacks {
        for col in 0..=slices {
            let theta = std::f32::consts::PI * row as f32 / stacks as f32;
            let phi = std::f32::consts::TAU * col as f32 / slices as f32;
            let p = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            vertices.push(SphereVertex {
                position: p,
                normal: p,
            });
        }
    }
    for row in 0..stacks {
        for col in 0..slices {
            let a = row * (slices + 1) + col;
            let b = a + slices + 1;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared marker sphere"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared marker indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        indices.len() as u32,
    )
}
