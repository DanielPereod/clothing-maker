//! Elección de la tarjeta gráfica.
//!
//! Se usa la mejor GPU que haya (la dedicada antes que la integrada, y
//! Vulkan, Metal o DirectX 12 antes que OpenGL). Si no hay ninguna, se
//! dibuja con la CPU (llvmpipe, lavapipe o WARP), más lento pero con la vista
//! 3D funcionando. Con `WGPU_ADAPTER_NAME` se puede forzar una concreta.

use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};

/// Cómo crear el dispositivo de wgpu, con nuestra forma de elegir la tarjeta.
pub fn setup() -> egui_wgpu::WgpuSetup {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.native_adapter_selector = Some(Arc::new(choose));
    egui_wgpu::WgpuSetup::CreateNew(setup)
}

fn choose(adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>) -> Result<wgpu::Adapter, String> {
    let usable: Vec<&wgpu::Adapter> =
        adapters.iter().filter(|a| surface.is_none_or(|s| a.is_surface_supported(s))).collect();
    let wanted = std::env::var("WGPU_ADAPTER_NAME").ok().map(|n| n.to_lowercase());
    let chosen = wanted
        .and_then(|name| usable.iter().find(|a| a.get_info().name.to_lowercase().contains(&name)))
        .or_else(|| {
            usable.iter().max_by_key(|a| {
                let i = a.get_info();
                rank(i.device_type, i.backend)
            })
        })
        .map(|a| (*a).clone())
        .ok_or_else(|| {
            "no hay ninguna tarjeta gráfica (ni dibujo por CPU) que pueda pintar en la ventana".to_owned()
        })?;
    for a in adapters {
        let info = a.get_info();
        let mark = if info == chosen.get_info() { "*" } else { " " };
        eprintln!("{mark} {}", describe(&info));
    }
    Ok(chosen)
}

/// Cuanto más alto, mejor.
fn rank(device_type: wgpu::DeviceType, backend: wgpu::Backend) -> (u8, u8) {
    let kind = match device_type {
        wgpu::DeviceType::DiscreteGpu => 4,
        wgpu::DeviceType::IntegratedGpu => 3,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 1,
        wgpu::DeviceType::Cpu => 0,
    };
    let backend = match backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal | wgpu::Backend::Dx12 => 1,
        _ => 0,
    };
    (kind, backend)
}

/// Si la tarjeta es de verdad o se dibuja con la CPU.
pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    info.device_type == wgpu::DeviceType::Cpu
}

pub fn describe(info: &wgpu::AdapterInfo) -> String {
    let kind = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => "GPU dedicada",
        wgpu::DeviceType::IntegratedGpu => "GPU integrada",
        wgpu::DeviceType::VirtualGpu => "GPU virtual",
        wgpu::DeviceType::Cpu => "CPU",
        wgpu::DeviceType::Other => "otra",
    };
    format!("{} ({kind}, {})", info.name, info.backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_real_gpus_and_modern_backends() {
        use wgpu::{Backend::*, DeviceType::*};
        let mut list =
            [(Cpu, Vulkan), (IntegratedGpu, Vulkan), (DiscreteGpu, Gl), (DiscreteGpu, Vulkan), (IntegratedGpu, Gl)];
        list.sort_by_key(|&(kind, backend)| std::cmp::Reverse(rank(kind, backend)));
        assert_eq!(
            list,
            [(DiscreteGpu, Vulkan), (DiscreteGpu, Gl), (IntegratedGpu, Vulkan), (IntegratedGpu, Gl), (Cpu, Vulkan)]
        );
    }
}
