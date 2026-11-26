//! Wire protocol, shared lockless ring buffers, and untrusted-input decoders
//! for the `apple-pvgpu` virtual GPU device.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod ops;
pub mod ring;
pub mod wire;
