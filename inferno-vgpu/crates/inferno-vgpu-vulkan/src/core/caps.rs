//! Device capabilities reported to the guest iOS shim.

/// Metal capabilities derived from the active `Platform` device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCaps {
    /// Human-readable GPU name.
    pub device_name: String,
    /// PCI vendor ID.
    pub vendor_id: u32,
    /// PCI device ID.
    pub device_id: u32,
    /// Whether the host GPU uses unified memory.
    pub has_unified_memory: bool,
    /// Maximum threadgroup memory length in bytes.
    pub max_threadgroup_memory_length: u32,
    /// Maximum buffer length in bytes.
    pub max_buffer_length: u64,
    /// Supported read-write texture tier (`0`, `1`, `2`).
    pub read_write_texture_tier: u32,
    /// Supported argument buffers tier (`0`, `1`, `2`).
    pub argument_buffers_tier: u32,
    /// Reported Apple GPU family (`1006` = Apple6 / A13).
    pub apple_gpu_family: u32,
    /// Whether function pointers are supported.
    pub supports_function_pointers: bool,
}

impl Default for DeviceCaps {
    fn default() -> Self {
        Self {
            device_name: String::from("Inferno Virtual GPU"),
            vendor_id: 0x106B,
            device_id: 0x8030,
            has_unified_memory: true,
            max_threadgroup_memory_length: 32768,
            max_buffer_length: 256 * 1024 * 1024,
            read_write_texture_tier: 2,
            argument_buffers_tier: 2,
            apple_gpu_family: 1006,
            supports_function_pointers: true,
        }
    }
}
