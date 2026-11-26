//! Scanout frame sink holding the latest presented BGRA8 framebuffer.

use parking_lot::RwLock;

/// Latest presented frame dimensions and BGRA8 pixels.
#[derive(Clone, Default)]
pub struct FrameBuffer {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// BGRA8 pixel bytes (`width * height * 4`).
    pub bgra8: Vec<u8>,
    /// Monotonic frame counter.
    pub frame_seq: u64,
}

/// Thread-safe scanout sink consumed by `read_display` (VNC / `apple_displaypipe_v4`)
/// and future direct Vulkan window swapchain presenters.
pub struct MemoryFrameSink {
    frame: RwLock<FrameBuffer>,
}

impl MemoryFrameSink {
    /// Creates a sink initialized to black at `(width, height)`.
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        Self {
            frame: RwLock::new(FrameBuffer {
                width,
                height,
                bgra8: vec![0u8; len],
                frame_seq: 0,
            }),
        }
    }

    /// Resizes the active display mode.
    pub fn set_mode(&self, width: u32, height: u32) {
        let mut guard = self.frame.write();
        guard.width = width;
        guard.height = height;
        let len = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        guard.bgra8.resize(len, 0);
    }

    /// Updates the presented frame from BGRA8 bytes.
    pub fn present_bgra8(&self, width: u32, height: u32, pixels: &[u8]) {
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        let mut guard = self.frame.write();
        guard.width = width;
        guard.height = height;
        guard.bgra8.resize(expected, 0);
        let copy_len = expected.min(pixels.len());
        guard.bgra8[..copy_len].copy_from_slice(&pixels[..copy_len]);
        guard.frame_seq = guard.frame_seq.wrapping_add(1);
    }

    /// Reads the latest presented frame into `dst`, returning `(width, height, frame_seq)`.
    pub fn read_into(&self, dst: &mut [u8]) -> Result<(u32, u32, u64), usize> {
        let guard = self.frame.read();
        let need = guard.bgra8.len();
        if dst.len() < need {
            return Err(need);
        }
        dst[..need].copy_from_slice(&guard.bgra8);
        Ok((guard.width, guard.height, guard.frame_seq))
    }

    /// Returns current `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        let guard = self.frame.read();
        (guard.width, guard.height)
    }
}
