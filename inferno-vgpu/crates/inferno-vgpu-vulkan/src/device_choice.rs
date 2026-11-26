//! Host Vulkan GPU selection over `metal2vulkan_runtime::floor`.

use ash::vk;
use metal2vulkan_runtime::{Instance, PhysicalDevice};

/// Probed physical device candidate passing `metal2vulkan-runtime`'s floor.
pub struct CandidateDevice {
    /// Physical device handle.
    pub physical: PhysicalDevice,
    /// Device name.
    pub name: String,
    /// Vendor ID.
    pub vendor_id: u32,
    /// Device ID.
    pub device_id: u32,
    /// Physical device type.
    pub kind: vk::PhysicalDeviceType,
}

/// Selects the best usable Vulkan GPU from `instance`, honoring `INFERNO_VGPU_DEVICE`
/// (either a numeric index or a case-insensitive device-name substring).
pub fn choose_physical_device(instance: &Instance) -> Result<CandidateDevice, String> {
    let physicals = instance
        .physical_devices()
        .map_err(|e| format!("vkEnumeratePhysicalDevices failed: {e}"))?;
    if physicals.is_empty() {
        return Err("no Vulkan physical devices found".to_owned());
    }

    let (_, vk_instance) = instance.raw();
    let mut usable = Vec::new();
    let mut rejected = Vec::new();

    for (idx, physical) in physicals.into_iter().enumerate() {
        // SAFETY: `physical` belongs to `vk_instance`.
        let props = unsafe { vk_instance.get_physical_device_properties(physical.raw()) };
        let name = props
            .device_name_as_c_str()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| format!("GPU#{idx}"));

        match physical.check() {
            Ok(_) => {
                log::info!(
                    "inferno-vgpu: Vulkan candidate [{idx}] {name} (vendor={:#06x} device={:#06x} type={:#x}) passed floor",
                    props.vendor_id,
                    props.device_id,
                    props.device_type.as_raw()
                );
                usable.push((
                    idx,
                    CandidateDevice {
                        physical,
                        name,
                        vendor_id: props.vendor_id,
                        device_id: props.device_id,
                        kind: props.device_type,
                    },
                ));
            }
            Err(err) => {
                log::warn!("inferno-vgpu: Vulkan device [{idx}] {name} rejected by floor: {err}");
                rejected.push(format!("{name}: {err}"));
            }
        }
    }

    if usable.is_empty() {
        return Err(format!(
            "no Vulkan device satisfied metal2vulkan-runtime floor ({})",
            rejected.join("; ")
        ));
    }

    if let Ok(sel) = std::env::var("INFERNO_VGPU_DEVICE") {
        let sel = sel.trim();
        if !sel.is_empty() {
            if let Ok(want_idx) = sel.parse::<usize>()
                && let Some((_, cand)) = usable.iter().find(|(idx, _)| *idx == want_idx)
            {
                return Ok(CandidateDevice {
                    physical: cand.physical.clone(),
                    name: cand.name.clone(),
                    vendor_id: cand.vendor_id,
                    device_id: cand.device_id,
                    kind: cand.kind,
                });
            }
            let lower = sel.to_lowercase();
            if let Some((_, cand)) = usable
                .iter()
                .find(|(_, c)| c.name.to_lowercase().contains(&lower))
            {
                return Ok(CandidateDevice {
                    physical: cand.physical.clone(),
                    name: cand.name.clone(),
                    vendor_id: cand.vendor_id,
                    device_id: cand.device_id,
                    kind: cand.kind,
                });
            }
            log::warn!(
                "INFERNO_VGPU_DEVICE={sel:?} did not match any usable Vulkan GPU; using automatic pick"
            );
        }
    }

    // Prefer discrete GPU, then integrated, then virtual, then CPU.
    let rank = |kind: vk::PhysicalDeviceType| -> u8 {
        match kind {
            vk::PhysicalDeviceType::DISCRETE_GPU => 0,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
            _ => 3,
        }
    };

    usable.sort_by_key(|(idx, c)| (rank(c.kind), *idx));
    let (_, best) = usable.remove(0);
    log::info!("inferno-vgpu: selected Vulkan GPU: {}", best.name);
    Ok(best)
}
