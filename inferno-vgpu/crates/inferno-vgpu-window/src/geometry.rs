//! Aspect-preserving display fit geometry and pointer coordinate mapping.

/// Fitted viewport rectangle inside a host window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FitRect {
    /// Left offset in host pixels.
    pub x: i32,
    /// Top offset in host pixels.
    pub y: i32,
    /// Width in host pixels.
    pub width: u32,
    /// Height in host pixels.
    pub height: u32,
}

/// Computes an aspect-preserving centered rectangle for guest `(guest_w, guest_h)`
/// inside host window `(win_w, win_h)`.
pub fn fit_aspect(win_w: u32, win_h: u32, guest_w: u32, guest_h: u32) -> FitRect {
    if win_w == 0 || win_h == 0 || guest_w == 0 || guest_h == 0 {
        return FitRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
    }
    let scale_w = (win_w as f64) / (guest_w as f64);
    let scale_h = (win_h as f64) / (guest_h as f64);
    let scale = scale_w.min(scale_h);
    let w = ((guest_w as f64) * scale).round().max(1.0) as u32;
    let h = ((guest_h as f64) * scale).round().max(1.0) as u32;
    let x = ((win_w.saturating_sub(w)) / 2) as i32;
    let y = ((win_h.saturating_sub(h)) / 2) as i32;
    FitRect {
        x,
        y,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iphone11_portrait_fits_inside_1080p() {
        let r = fit_aspect(1920, 1080, 828, 1792);
        assert_eq!(r.height, 1080);
        assert_eq!(r.width, 499);
        assert_eq!(r.y, 0);
        assert!(r.x > 0);
    }
}
