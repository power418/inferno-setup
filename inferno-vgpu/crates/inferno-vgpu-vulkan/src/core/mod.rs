//! Platform-independent core traits, device capabilities, error/refusal
//! accounting, and testkit.

pub mod caps;
pub mod memory;
pub mod platform;
pub mod refusal;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
