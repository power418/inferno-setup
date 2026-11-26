//! Public Rust API connecting the `inferno-vgpu-graphics` wire decoder and
//! shared rings to a `Platform` (`RtPlatform` on Vulkan or `FakePlatform` in tests).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod device;

pub use device::{DeviceConfig, GraphicsDevice};
pub use inferno_vgpu_vulkan::core::caps::DeviceCaps;
pub use inferno_vgpu_vulkan::core::platform::Platform;
pub use inferno_vgpu_vulkan::core::refusal::{AccuracyLedger, GpuError, RefusalCode};
pub use inferno_vgpu_vulkan::rt_platform::RtPlatform;
