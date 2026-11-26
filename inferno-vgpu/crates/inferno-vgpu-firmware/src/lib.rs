//! iOS kernelcache inspection, symbol import verification, kext prelink planning,
//! and `InfernoVgpu.fw` (`IVGF`) guest firmware/bundle package staging.
//!
//! Mirrors `reims-vgpu`'s `firmware/` + `guest/tools/{kc-imports,prebuilt}.py`
//! workflow for ChefKiss Inferno `t8030` (iOS 14+ `arm64e` kernelcaches).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod abi;
pub mod codesign;
pub mod im4p;
pub mod inject;
pub mod macho;
pub mod manifest;
