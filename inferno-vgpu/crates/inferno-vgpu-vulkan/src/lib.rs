//! Core `Platform` abstraction, Linux OS services, GPU device selection, and
//! `RtPlatform` implemented over `metal2vulkan` and `metal2vulkan-runtime`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod core;
pub mod device_choice;
pub mod os;
pub mod rt_platform;
