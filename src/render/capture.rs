use super::{GpuFrame, Renderer};
use anyhow::{Context, Result};
use std::{
    io::BufWriter,
    path::Path,
    sync::{Arc, mpsc},
    time::Duration,
};

/// Reuses one padded staging allocation. Preview never constructs this object.
#[derive(Default)]
pub struct Capture {
    owner: Option<Arc<()>>,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
}

impl Capture {
    pub fn read_rgba8(&mut self, renderer: &Renderer, frame: &GpuFrame) -> Result<Vec<u8>> {
        anyhow::ensure!(
            Arc::ptr_eq(&frame.targets.owner, &renderer.owner),
            "frame capture requires the renderer that owns the frame's GPU device"
        );
        // A Capture may be reused with another renderer, but its staging buffer
        // belongs to exactly one device, even when the required size is unchanged.
        if !self
            .owner
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, &renderer.owner))
        {
            self.buffer = None;
            self.capacity = 0;
            self.owner = Some(renderer.owner.clone());
        }
        let [width, height] = frame.size();
        let row_bytes = width * 4;
        let padded = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let required = u64::from(padded) * u64::from(height);
        if required > self.capacity {
            self.buffer = Some(renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture staging"),
                size: required,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.capacity = required;
        }
        let buffer = self.buffer.as_ref().unwrap();
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("explicit frame capture"),
            });
        encoder.copy_texture_to_buffer(
            frame.targets.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            frame.targets.texture.size(),
        );
        let submission = renderer.queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::sync_channel(1);
        buffer
            .slice(..required)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        renderer
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .context("wait for frame capture")?;
        receive
            .recv_timeout(Duration::from_secs(1))
            .context("receive capture mapping callback")??;
        let mapped = buffer.slice(..required).get_mapped_range()?;
        let mut pixels = Vec::with_capacity((row_bytes * height) as usize);
        for row in mapped.chunks_exact(padded as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }
}

pub fn write_png(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<()> {
    write_png_with_overwrite(path, size, pixels, true)
}

pub fn write_png_with_overwrite(
    path: &Path,
    size: [u32; 2],
    pixels: &[u8],
    overwrite: bool,
) -> Result<()> {
    anyhow::ensure!(
        pixels.len() as u64 == u64::from(size[0]) * u64::from(size[1]) * 4,
        "RGBA byte length does not match output size"
    );
    let output = crate::output::AtomicOutput::new(path, overwrite)?;
    let file = BufWriter::new(
        std::fs::File::create(&output.temporary)
            .with_context(|| format!("create PNG {}", path.display()))?,
    );
    let mut encoder = png::Encoder::new(file, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    output.commit()?;
    Ok(())
}
