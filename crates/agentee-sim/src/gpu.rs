use fieldgrid::gpu::{Options, Software};
use std::sync::OnceLock;

pub use fieldgrid::Gpu;

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

pub fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(|| {
        if std::env::var_os("AGENTEE_NO_GPU").is_some() {
            return None;
        }
        Gpu::open_blocking(&Options {
            label: "agentee-sim",
            backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12,
            software: Software::Refuse,
            features: wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
            ..Options::default()
        })
        .ok()
    })
    .as_ref()
}
