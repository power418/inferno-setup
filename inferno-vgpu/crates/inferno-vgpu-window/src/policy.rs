//! Display resolution & alignment policy for iPhone `t8030`.

/// Default iPhone 11 (`t8030`) display width in pixels.
pub const DEFAULT_WIDTH: u32 = 828;
/// Default iPhone 11 (`t8030`) display height in pixels.
pub const DEFAULT_HEIGHT: u32 = 1792;
/// Default display refresh rate in Hz.
pub const DEFAULT_REFRESH_HZ: u32 = 60;

/// Aligns a requested display mode cleanly (minimum 64x64, width multiple of 4, height even).
pub fn normalize_mode(width: u32, height: u32) -> (u32, u32) {
    let w = (width.max(64)) & !3;
    let h = (height.max(64)) & !1;
    (w, h)
}
