use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub struct GpuOptions {
    /// Case-insensitive adapter-name substring. An explicit miss never falls back.
    pub adapter: Option<String>,
    pub backend: Backend,
    pub allow_software: bool,
}

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum Backend {
    #[default]
    Vulkan,
    Gl,
    All,
}
impl Backend {
    pub fn flags(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Gl => wgpu::Backends::GL,
            Self::All => wgpu::Backends::all(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AdapterReport {
    pub index: usize,
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub vendor: u32,
    pub device: u32,
    pub software: bool,
    pub max_texture_dimension_2d: u32,
    pub max_buffer_size: u64,
    pub msaa4: bool,
}

pub fn instance(
    options: &GpuOptions,
    display: Option<winit::event_loop::OwnedDisplayHandle>,
) -> wgpu::Instance {
    let mut desc = match display {
        Some(display) => wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display)),
        None => wgpu::InstanceDescriptor::new_without_display_handle(),
    };
    desc.backends = options.backend.flags();
    wgpu::Instance::new(desc)
}

pub fn report(index: usize, adapter: &wgpu::Adapter) -> AdapterReport {
    let info = adapter.get_info();
    let limits = adapter.limits();
    let msaa4 = [super::COLOR_FORMAT, super::DEPTH_FORMAT].iter().all(|f| {
        adapter
            .get_texture_format_features(*f)
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
    });
    AdapterReport {
        index,
        name: info.name,
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        driver: info.driver,
        driver_info: info.driver_info,
        vendor: info.vendor,
        device: info.device,
        software: matches!(info.device_type, wgpu::DeviceType::Cpu),
        max_texture_dimension_2d: limits.max_texture_dimension_2d,
        max_buffer_size: limits.max_buffer_size,
        msaa4,
    }
}

pub async fn adapters(options: &GpuOptions) -> Vec<AdapterReport> {
    instance(options, None)
        .enumerate_adapters(options.backend.flags())
        .await
        .iter()
        .enumerate()
        .map(|(i, a)| report(i, a))
        .collect()
}

pub async fn select(
    instance: &wgpu::Instance,
    options: &GpuOptions,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter> {
    let adapters = instance.enumerate_adapters(options.backend.flags()).await;
    let mut candidates = Vec::new();
    for adapter in adapters {
        let info = adapter.get_info();
        let software = info.device_type == wgpu::DeviceType::Cpu;
        if let Some(name) = &options.adapter
            && !info.name.to_lowercase().contains(&name.to_lowercase())
        {
            continue;
        }
        if software && !options.allow_software {
            continue;
        }
        if surface.is_some_and(|s| !adapter.is_surface_supported(s)) {
            continue;
        }
        let rank = match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        };
        candidates.push((rank, adapter));
    }
    candidates.sort_by_key(|(rank, _)| *rank);
    if let Some((_, adapter)) = candidates.into_iter().next() {
        return Ok(adapter);
    }
    bail!(
        "no eligible {:?} adapter{}{}. Run `lemma adapters` to inspect devices. Use --allow-software explicitly for correctness checks on software Vulkan. Check GPU device access and the Vulkan loader/ICD on NixOS",
        options.backend,
        options
            .adapter
            .as_ref()
            .map(|s| format!(" matching {s:?}"))
            .unwrap_or_default(),
        if surface.is_some() {
            " capable of presenting to this window"
        } else {
            ""
        }
    )
}

pub async fn device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    let mut limits = wgpu::Limits::default();
    limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d.min(8192);
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("lemma shared device"),
            required_limits: limits,
            ..Default::default()
        })
        .await
        .context("create the selected GPU device")
}

pub fn validate_size(device: &wgpu::Device, size: [u32; 2]) -> Result<()> {
    let max = device.limits().max_texture_dimension_2d;
    ensure!(
        size.iter().all(|x| *x > 0 && *x <= max),
        "output size {size:?} exceeds adapter limit {max} or is zero"
    );
    Ok(())
}
