//! Safe raw-byte guest/shared-memory copy helpers.

/// Reads bytes from a raw pointer range without creating a long-lived Rust slice
/// over concurrently mutated shared memory.
///
/// # Safety
/// `src..src + len` must remain valid readable memory for the duration of the call.
pub unsafe fn copy_from_shared_raw(src: *const u8, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    if len > 0 {
        // SAFETY: caller guarantees `src..src + len` is mapped and readable.
        unsafe {
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = core::ptr::read_volatile(src.add(i));
            }
        }
    }
    out
}

/// Writes `src` bytes into shared memory at `dst` using volatile byte writes.
///
/// # Safety
/// `dst..dst + src.len()` must remain valid writable memory for the duration of the call.
pub unsafe fn copy_to_shared_raw(dst: *mut u8, src: &[u8]) {
    if !src.is_empty() {
        // SAFETY: caller guarantees `dst..dst + src.len()` is mapped and writable.
        unsafe {
            for (i, &b) in src.iter().enumerate() {
                core::ptr::write_volatile(dst.add(i), b);
            }
        }
    }
}
