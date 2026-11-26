//! Linux host services: `memfd_create` shared memory region, `eventfd`, `/dev/udmabuf`.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared memory region backed by an anonymous `memfd` and `MAP_SHARED` `mmap`.
///
/// The underlying file descriptor can be passed to QEMU/Inferno via `SCM_RIGHTS`
/// or mapped directly when statically linked.
pub struct RegionMemory {
    fd: OwnedFd,
    ptr: NonNull<u8>,
    size: usize,
}

// SAFETY: `RegionMemory` owns the mapping; concurrent guest access is handled via volatile/atomic accesses.
unsafe impl Send for RegionMemory {}
// SAFETY: `RegionMemory` owns the mapping; concurrent guest access is handled via volatile/atomic accesses.
unsafe impl Sync for RegionMemory {}

impl RegionMemory {
    /// Allocates a shared memory region of `size` bytes rounded up to page size.
    pub fn allocate(size: usize) -> io::Result<Self> {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
        let aligned = size.max(page_size).next_multiple_of(page_size);
        let name = CString::new("inferno-vgpu-region")
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // SAFETY: Valid C string and flags.
        let raw_fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if raw_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw_fd` is a newly created valid FD.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        // SAFETY: Valid FD and positive length.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), aligned as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: Valid FD and aligned length.
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                aligned,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if mapped == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let ptr = NonNull::new(mapped.cast::<u8>())
            .ok_or_else(|| io::Error::other("mmap returned null"))?;
        Ok(Self {
            fd,
            ptr,
            size: aligned,
        })
    }

    /// Raw base pointer of the shared mapping.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Size of the shared mapping in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Underlying `memfd` descriptor.
    pub fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Drop for RegionMemory {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` and `self.size` were produced by `mmap`.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast::<libc::c_void>(), self.size);
        }
    }
}

/// Non-blocking `eventfd` wrapper for worker wakeups.
pub struct EventFd {
    fd: OwnedFd,
}

impl EventFd {
    /// Creates a new `eventfd`.
    pub fn new() -> io::Result<Self> {
        // SAFETY: Standard Linux syscall with valid flags.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: Newly created valid FD.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(raw) },
        })
    }

    /// Increments the eventfd counter by 1.
    pub fn signal(&self) {
        let val: u64 = 1;
        // SAFETY: Valid FD and 8-byte buffer.
        let _ = unsafe {
            libc::write(
                self.fd.as_raw_fd(),
                (&val as *const u64).cast::<libc::c_void>(),
                std::mem::size_of::<u64>(),
            )
        };
    }

    /// Drains pending counts.
    pub fn drain(&self) {
        let mut val: u64 = 0;
        // SAFETY: Valid FD and 8-byte buffer.
        let _ = unsafe {
            libc::read(
                self.fd.as_raw_fd(),
                (&mut val as *mut u64).cast::<libc::c_void>(),
                std::mem::size_of::<u64>(),
            )
        };
    }
}

impl AsRawFd for EventFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Returns true if `/dev/udmabuf` exists and can be opened for zero-copy imports.
pub fn has_udmabuf() -> bool {
    let path = Path::new("/dev/udmabuf");
    path.exists() && std::fs::OpenOptions::new().read(true).write(true).open(path).is_ok()
}

/// Process-wide exit gate preventing new Vulkan submissions after teardown begins.
pub struct ExitGate {
    closed: AtomicBool,
}

impl ExitGate {
    /// Creates an open gate.
    pub const fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
        }
    }

    /// Marks the gate closed.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Returns whether the gate has closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl Default for ExitGate {
    fn default() -> Self {
        Self::new()
    }
}
