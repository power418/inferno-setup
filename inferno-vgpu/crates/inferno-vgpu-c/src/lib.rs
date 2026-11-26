//! C ABI (`ivg_*`) exporting `inferno-vgpu` for direct static/dynamic linking
//! by ChefKiss Inferno (`apple_pvgpu.c`).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use inferno_vgpu::{DeviceConfig, GraphicsDevice, RtPlatform};
use std::ffi::{c_char, c_void};
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Status code returned by `ivg_*` C functions.
pub type IvgStatus = u32;

/// Operation succeeded.
pub const IVG_OK: IvgStatus = 0;
/// Null pointer, bad `struct_size`, or invalid argument.
pub const IVG_ERR_INVALID_ARGUMENT: IvgStatus = 1;
/// Unsupported `struct_size` trailing bytes or platform feature.
pub const IVG_ERR_UNSUPPORTED: IvgStatus = 2;
/// Host memory, thread, or Vulkan GPU unavailable.
pub const IVG_ERR_HOST_RESOURCES: IvgStatus = 3;
/// GPU execution failure.
pub const IVG_ERR_GPU: IvgStatus = 4;
/// Internal panic caught at FFI boundary.
pub const IVG_ERR_INTERNAL: IvgStatus = 5;

/// Host interrupt raise callback.
pub type IvgRaiseIrqFn = Option<unsafe extern "C" fn(ctx: *mut c_void, irq_bits: u32)>;
/// Host callback context destructor.
pub type IvgDestroyFn = Option<unsafe extern "C" fn(ctx: *mut c_void)>;

/// Callback table passed to `ivg_graphics_device_create`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IvgGraphicsCallbacks {
    /// Size of this struct (`sizeof(IvgGraphicsCallbacks)`).
    pub struct_size: u32,
    /// Opaque host context pointer.
    pub ctx: *mut c_void,
    /// Callback invoked on host worker threads when device raises an IRQ.
    pub raise_irq: IvgRaiseIrqFn,
    /// Optional destructor called on `ivg_graphics_device_release`.
    pub destroy: IvgDestroyFn,
}

/// Configuration passed to `ivg_graphics_device_create`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IvgGraphicsDeviceConfig {
    /// Size of this struct (`sizeof(IvgGraphicsDeviceConfig)`).
    pub struct_size: u32,
    /// Shared Region 1 size in bytes (0 = default 64 MiB).
    pub region_size: u64,
    /// Initial display width in pixels (0 = 828).
    pub display_width: u32,
    /// Initial display height in pixels (0 = 1792).
    pub display_height: u32,
    /// Non-zero to force FakePlatform (only active when compiled with `testkit`).
    pub use_fake_platform: u32,
}

struct SendCtx(*mut c_void);
// SAFETY: Host callbacks (`raise_irq`, `destroy`) are thread-safe per C API contract.
unsafe impl Send for SendCtx {}
// SAFETY: Host callbacks (`raise_irq`, `destroy`) are thread-safe per C API contract.
unsafe impl Sync for SendCtx {}

struct CallbackGuard {
    ctx: SendCtx,
    destroy: IvgDestroyFn,
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        if let Some(f) = self.destroy {
            // SAFETY: `ctx` belongs to the caller and `destroy` runs once on device teardown.
            unsafe { f(self.ctx.0) };
        }
    }
}

enum BackendDevice {
    Vulkan(GraphicsDevice<RtPlatform>),
    #[cfg(any(test, feature = "testkit"))]
    Fake(GraphicsDevice<inferno_vgpu_vulkan::core::testkit::FakePlatform>),
}

/// Opaque C handle to a virtual graphics device.
pub struct IvgGraphicsDevice {
    backend: BackendDevice,
    _cb_guard: CallbackGuard,
}

/// Returns a static null-terminated description of `status`.
#[unsafe(no_mangle)]
pub extern "C" fn ivg_status_message(status: IvgStatus) -> *const c_char {
    let msg: &'static [u8] = match status {
        IVG_OK => b"ok\0",
        IVG_ERR_INVALID_ARGUMENT => b"invalid argument\0",
        IVG_ERR_UNSUPPORTED => b"unsupported\0",
        IVG_ERR_HOST_RESOURCES => b"host resources unavailable\0",
        IVG_ERR_GPU => b"gpu error\0",
        IVG_ERR_INTERNAL => b"internal error\0",
        _ => b"unknown status\0",
    };
    msg.as_ptr().cast::<c_char>()
}

/// Creates a graphics device instance.
///
/// # Safety
/// `config`, `callbacks` (if non-null), and `out_device` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_create(
    config: *const IvgGraphicsDeviceConfig,
    callbacks: *const IvgGraphicsCallbacks,
    out_device: *mut *mut IvgGraphicsDevice,
) -> IvgStatus {
    let res = catch_unwind(AssertUnwindSafe(|| {
        if out_device.is_null() || config.is_null() {
            return IVG_ERR_INVALID_ARGUMENT;
        }
        // SAFETY: Checked non-null above.
        unsafe { *out_device = std::ptr::null_mut() };

        // SAFETY: Caller guarantees `config` points to at least `struct_size` bytes.
        let cfg = unsafe { &*config };
        if (cfg.struct_size as usize) < size_of::<IvgGraphicsDeviceConfig>() {
            return IVG_ERR_INVALID_ARGUMENT;
        }

        let cb = if callbacks.is_null() {
            IvgGraphicsCallbacks {
                struct_size: size_of::<IvgGraphicsCallbacks>() as u32,
                ctx: std::ptr::null_mut(),
                raise_irq: None,
                destroy: None,
            }
        } else {
            // SAFETY: Caller guarantees `callbacks` is valid.
            let c = unsafe { &*callbacks };
            if (c.struct_size as usize) < size_of::<IvgGraphicsCallbacks>() {
                return IVG_ERR_INVALID_ARGUMENT;
            }
            *c
        };

        let ctx_addr = cb.ctx.addr();
        let raise_fn = cb.raise_irq;
        let raise_irq: Arc<dyn Fn(u32) + Send + Sync> = Arc::new(move |bits| {
            if let Some(f) = raise_fn {
                // SAFETY: `ctx_addr` comes from caller and outlives device until `CallbackGuard` drops.
                unsafe { f(std::ptr::without_provenance_mut(ctx_addr), bits) };
            }
        });

        let dev_cfg = DeviceConfig {
            region_size: if cfg.region_size == 0 {
                64 * 1024 * 1024
            } else {
                cfg.region_size as usize
            },
            display_width: if cfg.display_width == 0 {
                828
            } else {
                cfg.display_width
            },
            display_height: if cfg.display_height == 0 {
                1792
            } else {
                cfg.display_height
            },
            raise_irq,
        };

        let backend = if cfg.use_fake_platform != 0 {
            #[cfg(any(test, feature = "testkit"))]
            {
                let fake = Arc::new(inferno_vgpu_vulkan::core::testkit::FakePlatform::new());
                match GraphicsDevice::new(fake, dev_cfg) {
                    Ok(d) => BackendDevice::Fake(d),
                    Err(e) => {
                        log::error!("FakePlatform GraphicsDevice::new failed: {e}");
                        return IVG_ERR_HOST_RESOURCES;
                    }
                }
            }
            #[cfg(not(any(test, feature = "testkit")))]
            {
                return IVG_ERR_UNSUPPORTED;
            }
        } else {
            let rt = match RtPlatform::new(None) {
                Ok(rt) => Arc::new(rt),
                Err(e) => {
                    log::error!("RtPlatform::new failed: {e}");
                    return IVG_ERR_HOST_RESOURCES;
                }
            };
            match GraphicsDevice::new(rt, dev_cfg) {
                Ok(d) => BackendDevice::Vulkan(d),
                Err(e) => {
                    log::error!("GraphicsDevice::new failed: {e}");
                    return IVG_ERR_HOST_RESOURCES;
                }
            }
        };

        let boxed = Box::new(IvgGraphicsDevice {
            backend,
            _cb_guard: CallbackGuard {
                ctx: SendCtx(cb.ctx),
                destroy: cb.destroy,
            },
        });
        // SAFETY: `out_device` checked non-null.
        unsafe { *out_device = Box::into_raw(boxed) };
        IVG_OK
    }));

    res.unwrap_or(IVG_ERR_INTERNAL)
}

/// Releases a graphics device (no-op if null).
///
/// # Safety
/// `device` must be null or returned by `ivg_graphics_device_create` and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_release(device: *mut IvgGraphicsDevice) {
    if device.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Caller transfers ownership back to Rust for drop.
        drop(unsafe { Box::from_raw(device) });
    }));
}

/// Returns Region 1 shared memory pointer.
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_region_ptr(
    device: *const IvgGraphicsDevice,
) -> *mut c_void {
    if device.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Non-null device handle.
    let dev = unsafe { &*device };
    match &dev.backend {
        BackendDevice::Vulkan(d) => d.region_ptr().cast::<c_void>(),
        #[cfg(any(test, feature = "testkit"))]
        BackendDevice::Fake(d) => d.region_ptr().cast::<c_void>(),
    }
}

/// Returns Region 1 `memfd` file descriptor (`-1` on error).
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_region_fd(device: *const IvgGraphicsDevice) -> i32 {
    if device.is_null() {
        return -1;
    }
    // SAFETY: Non-null device handle.
    let dev = unsafe { &*device };
    match &dev.backend {
        BackendDevice::Vulkan(d) => d.region_fd(),
        #[cfg(any(test, feature = "testkit"))]
        BackendDevice::Fake(d) => d.region_fd(),
    }
}

/// Returns Region 1 size in bytes.
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_region_size(
    device: *const IvgGraphicsDevice,
) -> usize {
    if device.is_null() {
        return 0;
    }
    // SAFETY: Non-null device handle.
    let dev = unsafe { &*device };
    match &dev.backend {
        BackendDevice::Vulkan(d) => d.region_size(),
        #[cfg(any(test, feature = "testkit"))]
        BackendDevice::Fake(d) => d.region_size(),
    }
}

/// Reads a 32-bit MMIO register.
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_mmio_read(
    device: *mut IvgGraphicsDevice,
    offset: u64,
) -> u32 {
    if device.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Non-null device handle.
        let dev = unsafe { &*device };
        match &dev.backend {
            BackendDevice::Vulkan(d) => d.mmio_read(offset),
            #[cfg(any(test, feature = "testkit"))]
            BackendDevice::Fake(d) => d.mmio_read(offset),
        }
    }))
    .unwrap_or(0)
}

/// Writes a 32-bit MMIO register.
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_mmio_write(
    device: *mut IvgGraphicsDevice,
    offset: u64,
    value: u32,
) {
    if device.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Non-null device handle.
        let dev = unsafe { &*device };
        match &dev.backend {
            BackendDevice::Vulkan(d) => d.mmio_write(offset, value),
            #[cfg(any(test, feature = "testkit"))]
            BackendDevice::Fake(d) => d.mmio_write(offset, value),
        }
    }));
}

/// Rings a doorbell (`queue_id == 0` for control ring).
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_doorbell_write(
    device: *mut IvgGraphicsDevice,
    queue_id: u64,
) {
    if device.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Non-null device handle.
        let dev = unsafe { &*device };
        match &dev.backend {
            BackendDevice::Vulkan(d) => d.doorbell_write(queue_id),
            #[cfg(any(test, feature = "testkit"))]
            BackendDevice::Fake(d) => d.doorbell_write(queue_id),
        }
    }));
}

/// Resets the graphics device.
///
/// # Safety
/// `device` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_reset(device: *mut IvgGraphicsDevice) {
    if device.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Non-null device handle.
        let dev = unsafe { &*device };
        match &dev.backend {
            BackendDevice::Vulkan(d) => d.reset(),
            #[cfg(any(test, feature = "testkit"))]
            BackendDevice::Fake(d) => d.reset(),
        }
    }));
}

/// Copies the latest presented BGRA8 display frame into `out_bgra8`.
///
/// # Safety
/// `device`, `out_bgra8` (for `out_len` bytes), `out_width`, and `out_height` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ivg_graphics_device_read_display(
    device: *mut IvgGraphicsDevice,
    out_bgra8: *mut u8,
    out_len: usize,
    out_width: *mut u32,
    out_height: *mut u32,
) -> IvgStatus {
    if device.is_null() || out_bgra8.is_null() {
        return IVG_ERR_INVALID_ARGUMENT;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Non-null pointers checked above.
        let dev = unsafe { &*device };
        // SAFETY: Caller promises `out_bgra8..out_bgra8 + out_len` is writable.
        let dst = unsafe { std::slice::from_raw_parts_mut(out_bgra8, out_len) };
        let res = match &dev.backend {
            BackendDevice::Vulkan(d) => d.read_display(dst),
            #[cfg(any(test, feature = "testkit"))]
            BackendDevice::Fake(d) => d.read_display(dst),
        };
        match res {
            Ok((w, h, _)) => {
                if !out_width.is_null() {
                    // SAFETY: Checked non-null.
                    unsafe { *out_width = w };
                }
                if !out_height.is_null() {
                    // SAFETY: Checked non-null.
                    unsafe { *out_height = h };
                }
                IVG_OK
            }
            Err(_) => IVG_ERR_INVALID_ARGUMENT,
        }
    }))
    .unwrap_or(IVG_ERR_INTERNAL)
}

/// Quiesces global runtime state before host process exit.
#[unsafe(no_mangle)]
pub extern "C" fn ivg_quiesce_for_exit() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use inferno_vgpu_graphics::wire::{IVG_MAGIC, regs};

    #[test]
    fn c_api_fake_device_lifecycle() {
        let cfg = IvgGraphicsDeviceConfig {
            struct_size: size_of::<IvgGraphicsDeviceConfig>() as u32,
            region_size: 4 * 1024 * 1024,
            display_width: 828,
            display_height: 1792,
            use_fake_platform: 1,
        };
        let mut dev: *mut IvgGraphicsDevice = std::ptr::null_mut();
        let st = unsafe { ivg_graphics_device_create(&cfg, std::ptr::null(), &mut dev) };
        assert_eq!(st, IVG_OK);
        assert!(!dev.is_null());
        assert_eq!(
            unsafe { ivg_graphics_device_mmio_read(dev, regs::MAGIC) },
            IVG_MAGIC
        );
        assert!(unsafe { ivg_graphics_device_region_fd(dev) } >= 0);
        unsafe { ivg_graphics_device_release(dev) };
    }
}
