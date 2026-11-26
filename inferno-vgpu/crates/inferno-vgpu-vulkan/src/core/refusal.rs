//! Structured error, refusal codes, and AccuracyLedger counters.

use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::fmt;

/// Numeric refusal codes transmitted back to guest in `IvgCompletionRecord::refusal_code`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalCode {
    /// No refusal.
    None = 0,
    /// Invalid wire argument or unknown enum ordinal.
    InvalidDescriptor = 1,
    /// Metal API validation refusal (e.g. out-of-range offset, missing pipeline).
    MetalRefusal = 2,
    /// Feature not yet implemented or unsupported by host Vulkan / `metal2vulkan-runtime`.
    UnsupportedFeature = 3,
    /// Shader translation refusal (`metal2vulkan` AIR -> SPIR-V).
    ShaderTranslation = 4,
    /// Host Vulkan driver error or out of memory.
    VulkanDriver = 5,
    /// Device lost or quiesced.
    DeviceLost = 6,
    /// Missing handle in context resource table.
    UnknownHandle = 7,
}

/// Error returned by `Platform` operations.
#[derive(Clone, Debug)]
pub struct GpuError {
    /// Wire refusal code.
    pub code: RefusalCode,
    /// Metal command buffer error status (`1 = Internal`, `2 = Timeout`, `4 = PageFault`, `7 = NotPermitted`, `12 = InvalidResource`, etc.).
    pub metal_status: u32,
    /// Human-readable reason for host logs and accuracy tracking.
    pub message: String,
}

impl GpuError {
    /// Constructs a new `GpuError`.
    pub fn new(code: RefusalCode, metal_status: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            metal_status,
            message: message.into(),
        }
    }

    /// Invalid descriptor or raw Metal value.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(RefusalCode::InvalidDescriptor, 1, msg)
    }

    /// Unsupported feature / `metal2vulkan` gap.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::new(RefusalCode::UnsupportedFeature, 1, msg)
    }

    /// Unknown handle.
    pub fn unknown_handle(msg: impl Into<String>) -> Self {
        Self::new(RefusalCode::UnknownHandle, 12, msg)
    }
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}

impl std::error::Error for GpuError {}

/// Tracks counts of structured refusals and gaps for diagnostics & Accuracy Ledger reporting.
#[derive(Default)]
pub struct AccuracyLedger {
    counts: Mutex<BTreeMap<(RefusalCode, String), u64>>,
}

impl AccuracyLedger {
    /// Records a refusal event and logs the first occurrence at warning level.
    pub fn record(&self, err: &GpuError) {
        let mut map = self.counts.lock();
        let key = (err.code, err.message.clone());
        let entry = map.entry(key).or_insert(0);
        *entry += 1;
        if *entry == 1 {
            log::warn!("inferno-vgpu refusal {:?}: {}", err.code, err.message);
        }
    }

    /// Returns a snapshot of recorded refusals.
    pub fn snapshot(&self) -> Vec<(RefusalCode, String, u64)> {
        self.counts
            .lock()
            .iter()
            .map(|((code, msg), &count)| (*code, msg.clone(), count))
            .collect()
    }
}
