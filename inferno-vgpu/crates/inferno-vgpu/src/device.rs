//! Virtual GPU device state machine, control & per-queue workers, and resource table.

use inferno_vgpu_graphics::ops::{ControlOp, QueueOp, decode_control, decode_queue};
use inferno_vgpu_graphics::ring::SharedRing;
use inferno_vgpu_graphics::wire::*;
use inferno_vgpu_vulkan::core::memory::{copy_from_shared_raw, copy_to_shared_raw};
use inferno_vgpu_vulkan::core::platform::{
    MemoryClass, Platform, RenderColorAttachmentDesc, RenderPassColorAttachment, RenderPassDesc,
    RenderPipelineDesc, ScissorRect, TextureDesc, Viewport,
};
use inferno_vgpu_vulkan::core::refusal::{GpuError, RefusalCode};
use inferno_vgpu_vulkan::os::linux::{EventFd, RegionMemory};
use inferno_vgpu_window::policy::{DEFAULT_HEIGHT, DEFAULT_WIDTH, normalize_mode};
use inferno_vgpu_window::present::MemoryFrameSink;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use zerocopy::IntoBytes;

/// Configuration for creating a [`GraphicsDevice`].
pub struct DeviceConfig {
    /// Shared Region 1 size in bytes (default: 64 MiB).
    pub region_size: usize,
    /// Initial display width in pixels.
    pub display_width: u32,
    /// Initial display height in pixels.
    pub display_height: u32,
    /// Callback invoked when the device raises an interrupt bitmask (`IVG_IRQ_*`).
    pub raise_irq: Arc<dyn Fn(u32) + Send + Sync>,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            region_size: 64 * 1024 * 1024,
            display_width: DEFAULT_WIDTH,
            display_height: DEFAULT_HEIGHT,
            raise_irq: Arc::new(|_| {}),
        }
    }
}

struct MmioRegs {
    features: u64,
    status: AtomicU32,
    control_sub_off: AtomicU64,
    control_cmp_off: AtomicU64,
    irq_status: AtomicU32,
    display_width: AtomicU32,
    display_height: AtomicU32,
    error_code: AtomicU32,
    region_size: u64,
}

struct QueueWorkerHandle {
    wake: Arc<EventFd>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct ContextResources<P: Platform> {
    blocks: HashMap<u64, P::Block>,
    buffers: HashMap<u64, P::Buffer>,
    textures: HashMap<u64, P::Texture>,
    libraries: HashMap<u64, P::Library>,
    compute_pipelines: HashMap<u64, P::ComputePipeline>,
    render_pipelines: HashMap<u64, P::RenderPipeline>,
    queues: HashMap<u64, (P::Queue, QueueWorkerHandle)>,
    events: HashMap<u64, P::Event>,
    fences: HashMap<u64, P::Fence>,
}

impl<P: Platform> Default for ContextResources<P> {
    fn default() -> Self {
        Self {
            blocks: HashMap::new(),
            buffers: HashMap::new(),
            textures: HashMap::new(),
            libraries: HashMap::new(),
            compute_pipelines: HashMap::new(),
            render_pipelines: HashMap::new(),
            queues: HashMap::new(),
            events: HashMap::new(),
            fences: HashMap::new(),
        }
    }
}

struct DeviceShared<P: Platform> {
    platform: Arc<P>,
    region: RegionMemory,
    mmio: MmioRegs,
    frame_sink: MemoryFrameSink,
    raise_irq: Arc<dyn Fn(u32) + Send + Sync>,
    contexts: Mutex<HashMap<u32, ContextResources<P>>>,
    library_cache: Mutex<HashMap<[u8; 32], P::Library>>,
    surfaces: Mutex<HashMap<u32, P::Texture>>,
    queue_wakes: Mutex<HashMap<u64, Arc<EventFd>>>,
    shutdown: AtomicBool,
}

/// Core virtual GPU device driving a `Platform` from `apple-pvgpu` MMIO and rings.
pub struct GraphicsDevice<P: Platform> {
    shared: Arc<DeviceShared<P>>,
    ctrl_wake: Arc<EventFd>,
    ctrl_thread: Option<JoinHandle<()>>,
}

impl<P: Platform> GraphicsDevice<P> {
    /// Creates a new `GraphicsDevice` on `platform` and starts the control worker.
    pub fn new(platform: Arc<P>, config: DeviceConfig) -> Result<Self, String> {
        let region = RegionMemory::allocate(config.region_size)
            .map_err(|e| format!("allocate shared region memfd: {e}"))?;
        let ctrl_wake =
            Arc::new(EventFd::new().map_err(|e| format!("create control eventfd: {e}"))?);

        let (w, h) = normalize_mode(config.display_width, config.display_height);
        let caps = platform.caps();
        let mut features = IVG_FEAT_DISPLAY_READBACK | IVG_FEAT_SURFACE_PAGES | IVG_FEAT_SHARED_POOL;
        if caps.has_unified_memory {
            features |= IVG_FEAT_UNIFIED_MEMORY;
        }

        let shared = Arc::new(DeviceShared {
            platform,
            mmio: MmioRegs {
                features,
                status: AtomicU32::new(IVG_STATUS_HOST_READY),
                control_sub_off: AtomicU64::new(0),
                control_cmp_off: AtomicU64::new(0),
                irq_status: AtomicU32::new(0),
                display_width: AtomicU32::new(w),
                display_height: AtomicU32::new(h),
                error_code: AtomicU32::new(0),
                region_size: region.size() as u64,
            },
            region,
            frame_sink: MemoryFrameSink::new(w, h),
            raise_irq: config.raise_irq,
            contexts: Mutex::new(HashMap::new()),
            library_cache: Mutex::new(HashMap::new()),
            surfaces: Mutex::new(HashMap::new()),
            queue_wakes: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
        });

        let thread_shared = Arc::clone(&shared);
        let thread_wake = Arc::clone(&ctrl_wake);
        let ctrl_thread = thread::Builder::new()
            .name("ivg-control".to_owned())
            .spawn(move || control_worker_loop(&thread_shared, &thread_wake))
            .map_err(|e| format!("spawn control thread: {e}"))?;

        Ok(Self {
            shared,
            ctrl_wake,
            ctrl_thread: Some(ctrl_thread),
        })
    }

    /// Raw pointer to Region 1 shared memory.
    pub fn region_ptr(&self) -> *mut u8 {
        self.shared.region.as_mut_ptr()
    }

    /// File descriptor of Region 1 `memfd`.
    pub fn region_fd(&self) -> RawFd {
        self.shared.region.raw_fd()
    }

    /// Size of Region 1 in bytes.
    pub fn region_size(&self) -> usize {
        self.shared.region.size()
    }

    /// Returns a reference to the backing platform.
    pub fn platform(&self) -> &Arc<P> {
        &self.shared.platform
    }

    /// Handles a 32-bit MMIO read from Region 0.
    pub fn mmio_read(&self, offset: u64) -> u32 {
        let m = &self.shared.mmio;
        match offset {
            regs::MAGIC => IVG_MAGIC,
            regs::VERSION => IVG_VERSION,
            regs::FEATURES_LO => m.features as u32,
            regs::FEATURES_HI => (m.features >> 32) as u32,
            regs::STATUS => m.status.load(Ordering::Acquire),
            regs::CONTROL_SUB_OFF_LO => m.control_sub_off.load(Ordering::Acquire) as u32,
            regs::CONTROL_SUB_OFF_HI => (m.control_sub_off.load(Ordering::Acquire) >> 32) as u32,
            regs::CONTROL_CMP_OFF_LO => m.control_cmp_off.load(Ordering::Acquire) as u32,
            regs::CONTROL_CMP_OFF_HI => (m.control_cmp_off.load(Ordering::Acquire) >> 32) as u32,
            regs::IRQ_STATUS => m.irq_status.load(Ordering::Acquire),
            regs::REGION_SIZE_LO => m.region_size as u32,
            regs::REGION_SIZE_HI => (m.region_size >> 32) as u32,
            regs::POOL_BASE_LO | regs::POOL_BASE_HI | regs::POOL_SIZE_LO | regs::POOL_SIZE_HI => 0,
            regs::DISPLAY_WIDTH => m.display_width.load(Ordering::Acquire),
            regs::DISPLAY_HEIGHT => m.display_height.load(Ordering::Acquire),
            regs::ERROR_CODE => m.error_code.load(Ordering::Acquire),
            _ => 0,
        }
    }

    /// Handles a 32-bit MMIO write to Region 0.
    pub fn mmio_write(&self, offset: u64, value: u32) {
        let m = &self.shared.mmio;
        match offset {
            regs::STATUS => {
                if value == IVG_STATUS_RESET {
                    self.reset();
                } else {
                    m.status
                        .store(value | IVG_STATUS_HOST_READY, Ordering::Release);
                    self.ctrl_wake.signal();
                }
            }
            regs::CONTROL_SUB_OFF_LO => {
                update_lo(&m.control_sub_off, value);
            }
            regs::CONTROL_SUB_OFF_HI => {
                update_hi(&m.control_sub_off, value);
            }
            regs::CONTROL_CMP_OFF_LO => {
                update_lo(&m.control_cmp_off, value);
            }
            regs::CONTROL_CMP_OFF_HI => {
                update_hi(&m.control_cmp_off, value);
            }
            regs::CONTROL_DOORBELL => {
                self.ctrl_wake.signal();
            }
            regs::QUEUE_DOORBELL => {
                self.doorbell_write(value as u64);
            }
            regs::IRQ_ACK => {
                m.irq_status.fetch_and(!value, Ordering::Release);
            }
            _ => {}
        }
    }

    /// Wakes the worker thread for `queue_id` (or control ring if `queue_id == 0`).
    pub fn doorbell_write(&self, queue_id: u64) {
        if queue_id == 0 {
            self.ctrl_wake.signal();
            return;
        }
        if let Some(wake) = self.shared.queue_wakes.lock().get(&queue_id).cloned() {
            wake.signal();
        }
    }

    /// Resets contexts, queues, and status registers cleanly.
    pub fn reset(&self) {
        let mut contexts = self.shared.contexts.lock();
        for (_, mut ctx) in contexts.drain() {
            for (_, (_, mut worker)) in ctx.queues.drain() {
                worker.stop.store(true, Ordering::SeqCst);
                worker.wake.signal();
                if let Some(handle) = worker.thread.take() {
                    let _ = handle.join();
                }
            }
        }
        self.shared.queue_wakes.lock().clear();
        self.shared.surfaces.lock().clear();
        self.shared
            .mmio
            .status
            .store(IVG_STATUS_HOST_READY, Ordering::Release);
        self.shared.mmio.irq_status.store(0, Ordering::Release);
        self.shared.mmio.error_code.store(0, Ordering::Release);
    }

    /// Reads the latest presented BGRA8 frame into `dst`.
    /// Returns `(width, height, frame_seq)` on success, or required byte length on `Err`.
    pub fn read_display(&self, dst: &mut [u8]) -> Result<(u32, u32, u64), usize> {
        self.shared.frame_sink.read_into(dst)
    }

    /// Marks the device lost (e.g., on host crash/disconnect simulation) and raises `IVG_IRQ_ERROR`.
    pub fn mark_device_lost(&self, code: u32) {
        self.shared.mmio.error_code.store(code, Ordering::Release);
        self.shared
            .mmio
            .status
            .fetch_or(IVG_STATUS_DEVICE_LOST, Ordering::Release);
        raise_irq_bit(&self.shared, IVG_IRQ_ERROR);
    }
}

impl<P: Platform> Drop for GraphicsDevice<P> {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.ctrl_wake.signal();
        if let Some(handle) = self.ctrl_thread.take() {
            let _ = handle.join();
        }
        self.reset();
        self.shared.platform.quiesce();
    }
}

fn update_lo(atom: &AtomicU64, lo: u32) {
    let cur = atom.load(Ordering::Acquire);
    atom.store((cur & !0xFFFF_FFFF) | u64::from(lo), Ordering::Release);
}

fn update_hi(atom: &AtomicU64, hi: u32) {
    let cur = atom.load(Ordering::Acquire);
    atom.store(
        (cur & 0xFFFF_FFFF) | (u64::from(hi) << 32),
        Ordering::Release,
    );
}

fn raise_irq_bit<P: Platform>(shared: &DeviceShared<P>, bit: u32) {
    shared.mmio.irq_status.fetch_or(bit, Ordering::Release);
    (shared.raise_irq)(bit);
}

fn wait_eventfd_or_timeout(ev: &EventFd, timeout_ms: i32) {
    let mut pfd = libc::pollfd {
        fd: ev.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `pfd` points to valid struct for 1 FD.
    let _ = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    ev.drain();
}

fn control_worker_loop<P: Platform>(shared: &Arc<DeviceShared<P>>, wake: &EventFd) {
    let mut scratch = Vec::with_capacity(4096);
    while !shared.shutdown.load(Ordering::Acquire) {
        let sub_off = shared.mmio.control_sub_off.load(Ordering::Acquire) as usize;
        let cmp_off = shared.mmio.control_cmp_off.load(Ordering::Acquire) as usize;
        let status = shared.mmio.status.load(Ordering::Acquire);

        if (status & IVG_STATUS_DRIVER_READY) != 0 && sub_off != cmp_off {
            let base = shared.region.as_mut_ptr();
            let len = shared.region.size();
            // SAFETY: `shared.region` outlives this thread (joined in Drop).
            if let (Ok(sub_ring), Ok(cmp_ring)) = unsafe {
                (
                    SharedRing::new(base, len, sub_off),
                    SharedRing::new(base, len, cmp_off),
                )
            } {
                sub_ring.set_host_idle(false);
                let mut processed = false;
                while !shared.shutdown.load(Ordering::Acquire) {
                    match sub_ring.try_pop_record(&mut scratch) {
                        Ok(Some(rec_len)) => {
                            processed = true;
                            let completion = handle_control_record(shared, &scratch[..rec_len]);
                            let _ = cmp_ring.push_bytes(completion.as_bytes());
                            raise_irq_bit(shared, IVG_IRQ_CONTROL);
                        }
                        Ok(None) => break,
                        Err(e) => {
                            log::warn!("control ring error: {e:?}");
                            break;
                        }
                    }
                }
                sub_ring.set_host_idle(true);
                if processed {
                    continue;
                }
            }
        }

        wait_eventfd_or_timeout(wake, 25);
    }
}

fn handle_control_record<P: Platform>(shared: &Arc<DeviceShared<P>>, raw: &[u8]) -> IvgCompletionRecord {
    let (hdr, op) = match decode_control(raw) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("decode_control failed: {e:?}");
            return IvgCompletionRecord {
                seqno: 0,
                status: 1,
                refusal_code: RefusalCode::InvalidDescriptor as u32,
                gpu_start_ns: 0,
                gpu_end_ns: 0,
                result_u64: 0,
            };
        }
    };

    let res = execute_control_op(shared, hdr.context_id, op);
    match res {
        Ok(val) => IvgCompletionRecord {
            seqno: hdr.seqno,
            status: 0,
            refusal_code: 0,
            gpu_start_ns: 0,
            gpu_end_ns: 0,
            result_u64: val,
        },
        Err(err) => {
            shared.platform.ledger().record(&err);
            IvgCompletionRecord {
                seqno: hdr.seqno,
                status: err.metal_status,
                refusal_code: err.code as u32,
                gpu_start_ns: 0,
                gpu_end_ns: 0,
                result_u64: 0,
            }
        }
    }
}

fn validate_region_slice(region_len: usize, offset: u64, byte_len: u64) -> Result<(usize, usize), GpuError> {
    let off = usize::try_from(offset).map_err(|_| GpuError::invalid("region offset overflow"))?;
    let len = usize::try_from(byte_len).map_err(|_| GpuError::invalid("region len overflow"))?;
    let end = off
        .checked_add(len)
        .ok_or_else(|| GpuError::invalid("region range overflow"))?;
    if end > region_len {
        return Err(GpuError::invalid("region range out of bounds"));
    }
    Ok((off, len))
}

fn execute_control_op<P: Platform>(
    shared: &Arc<DeviceShared<P>>,
    ctx_id: u32,
    op: ControlOp,
) -> Result<u64, GpuError> {
    let base = shared.region.as_mut_ptr();
    let region_len = shared.region.size();

    match op {
        ControlOp::Ping(p) => Ok(p.cookie),
        ControlOp::GetDeviceCaps(p) => {
            let (off, _) =
                validate_region_slice(region_len, p.caps_region_offset, std::mem::size_of::<IvgDeviceCaps>() as u64)?;
            let caps = shared.platform.caps();
            let mut wire = IvgDeviceCaps {
                vendor_id: caps.vendor_id,
                device_id: caps.device_id,
                has_unified_memory: u32::from(caps.has_unified_memory),
                max_threadgroup_memory_length: caps.max_threadgroup_memory_length,
                max_buffer_length: caps.max_buffer_length,
                read_write_texture_tier: caps.read_write_texture_tier,
                argument_buffers_tier: caps.argument_buffers_tier,
                apple_gpu_family: caps.apple_gpu_family,
                supports_function_pointers: u32::from(caps.supports_function_pointers),
                device_name: [0u8; 128],
            };
            let name_bytes = caps.device_name.as_bytes();
            let copy_len = name_bytes.len().min(127);
            wire.device_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            // SAFETY: `off + size_of::<IvgDeviceCaps>() <= region_len`.
            unsafe { copy_to_shared_raw(base.add(off), wire.as_bytes()) };
            Ok(0)
        }
        ControlOp::ContextCreate(_) => {
            shared.contexts.lock().entry(ctx_id).or_default();
            Ok(u64::from(ctx_id))
        }
        ControlOp::ContextDestroy => {
            let removed = shared.contexts.lock().remove(&ctx_id);
            if let Some(mut ctx) = removed {
                let mut wakes = shared.queue_wakes.lock();
                for (qid, (_, mut worker)) in ctx.queues.drain() {
                    wakes.remove(&qid);
                    worker.stop.store(true, Ordering::SeqCst);
                    worker.wake.signal();
                    if let Some(h) = worker.thread.take() {
                        let _ = h.join();
                    }
                }
            }
            Ok(0)
        }
        ControlOp::QueueCreate(p) => {
            let _ = validate_region_slice(region_len, p.sub_ring_offset, 64)?;
            let _ = validate_region_slice(region_len, p.cmp_ring_offset, 64)?;
            // SAFETY: Offsets validated within `shared.region`.
            let sub_ring = unsafe { SharedRing::new(base, region_len, p.sub_ring_offset as usize) }
                .map_err(|e| GpuError::invalid(format!("bad queue sub ring: {e:?}")))?;
            // SAFETY: Offsets validated within `shared.region`.
            let cmp_ring = unsafe { SharedRing::new(base, region_len, p.cmp_ring_offset as usize) }
                .map_err(|e| GpuError::invalid(format!("bad queue cmp ring: {e:?}")))?;

            let queue = shared
                .platform
                .new_command_queue(p.max_command_buffers.max(1))?;
            let wake = Arc::new(
                EventFd::new().map_err(|e| GpuError::new(RefusalCode::VulkanDriver, 1, e.to_string()))?,
            );
            let stop = Arc::new(AtomicBool::new(false));
            let worker_shared = Arc::clone(shared);
            let worker_wake = Arc::clone(&wake);
            let worker_stop = Arc::clone(&stop);
            let qid = p.queue_id;
            let handle = thread::Builder::new()
                .name(format!("ivg-q-{qid}"))
                .spawn(move || {
                    queue_worker_loop(
                        &worker_shared,
                        ctx_id,
                        qid,
                        sub_ring,
                        cmp_ring,
                        &worker_wake,
                        &worker_stop,
                    )
                })
                .map_err(|e| GpuError::new(RefusalCode::VulkanDriver, 1, e.to_string()))?;

            shared.queue_wakes.lock().insert(qid, Arc::clone(&wake));
            let mut contexts = shared.contexts.lock();
            let ctx = contexts.entry(ctx_id).or_default();
            ctx.queues.insert(
                qid,
                (
                    queue,
                    QueueWorkerHandle {
                        wake,
                        stop,
                        thread: Some(handle),
                    },
                ),
            );
            Ok(qid)
        }
        ControlOp::QueueDestroy(p) => {
            shared.queue_wakes.lock().remove(&p.handle);
            let worker = shared
                .contexts
                .lock()
                .get_mut(&ctx_id)
                .and_then(|c| c.queues.remove(&p.handle));
            if let Some((_, mut w)) = worker {
                w.stop.store(true, Ordering::SeqCst);
                w.wake.signal();
                if let Some(h) = w.thread.take() {
                    let _ = h.join();
                }
            }
            Ok(0)
        }
        ControlOp::BlockAlloc(p) => {
            let class = if p.memory_class == 1 {
                MemoryClass::Private
            } else {
                MemoryClass::Shared
            };
            let block = shared.platform.alloc_block(class, p.size)?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .blocks
                .insert(p.block_id, block);
            Ok(p.block_id)
        }
        ControlOp::BlockFree(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.blocks.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::BufferCreate(p) => {
            let block = shared
                .contexts
                .lock()
                .get(&ctx_id)
                .and_then(|c| c.blocks.get(&p.block_id).cloned())
                .ok_or_else(|| GpuError::unknown_handle("unknown block_id"))?;
            let buf = shared
                .platform
                .new_buffer(&block, p.block_offset, p.length)?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .buffers
                .insert(p.buffer_id, buf);
            Ok(p.buffer_id)
        }
        ControlOp::BufferDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.buffers.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::BufferUpload(p) => {
            let (off, len) = validate_region_slice(region_len, p.region_offset, p.length)?;
            // SAFETY: Range validated inside `shared.region`.
            let data = unsafe { copy_from_shared_raw(base.add(off), len) };
            let buf = get_buffer(shared, ctx_id, p.buffer_id)?;
            shared.platform.write_buffer(&buf, p.buffer_offset, &data)?;
            Ok(0)
        }
        ControlOp::BufferReadback(p) => {
            let (off, len) = validate_region_slice(region_len, p.region_offset, p.length)?;
            let buf = get_buffer(shared, ctx_id, p.buffer_id)?;
            let mut out = vec![0u8; len];
            shared
                .platform
                .read_buffer(&buf, p.buffer_offset, &mut out)?;
            // SAFETY: Range validated inside `shared.region`.
            unsafe { copy_to_shared_raw(base.add(off), &out) };
            Ok(0)
        }
        ControlOp::TextureCreate(p) => {
            let desc = wire_to_tex_desc(&p.desc);
            let tex = shared.platform.new_texture(&desc)?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .textures
                .insert(p.texture_id, tex);
            Ok(p.texture_id)
        }
        ControlOp::TextureDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.textures.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::TextureUpload(p) => {
            let total_bytes = p
                .bytes_per_image
                .max(p.bytes_per_row.saturating_mul(u64::from(p.extent[1].max(1))));
            let (off, len) = validate_region_slice(region_len, p.region_offset, total_bytes)?;
            // SAFETY: Range validated inside `shared.region`.
            let data = unsafe { copy_from_shared_raw(base.add(off), len) };
            let tex = get_texture(shared, ctx_id, p.texture_id)?;
            shared.platform.texture_replace_region(
                &tex,
                p.origin,
                p.extent,
                p.level,
                p.slice,
                &data,
                p.bytes_per_row,
                p.bytes_per_image,
            )?;
            Ok(0)
        }
        ControlOp::TextureReadback(p) => {
            let total_bytes = p
                .bytes_per_image
                .max(p.bytes_per_row.saturating_mul(u64::from(p.extent[1].max(1))));
            let (off, len) = validate_region_slice(region_len, p.region_offset, total_bytes)?;
            let tex = get_texture(shared, ctx_id, p.texture_id)?;
            let mut out = vec![0u8; len];
            shared.platform.texture_get_bytes(
                &tex,
                p.origin,
                p.extent,
                p.level,
                p.slice,
                &mut out,
                p.bytes_per_row,
                p.bytes_per_image,
            )?;
            // SAFETY: Range validated inside `shared.region`.
            unsafe { copy_to_shared_raw(base.add(off), &out) };
            Ok(0)
        }
        ControlOp::LibraryCreate(p) => {
            let (off, len) = validate_region_slice(region_len, p.region_offset, p.byte_len)?;
            let cached = shared.library_cache.lock().get(&p.sha256).cloned();
            let lib = if let Some(hit) = cached {
                hit
            } else {
                // SAFETY: Range validated inside `shared.region`.
                let bytes = unsafe { copy_from_shared_raw(base.add(off), len) };
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                let parsed = shared.platform.new_library(&bytes)?;
                let mut cache = shared.library_cache.lock();
                cache.insert(digest, parsed.clone());
                cache.insert(p.sha256, parsed.clone());
                parsed
            };
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .libraries
                .insert(p.library_id, lib);
            Ok(p.library_id)
        }
        ControlOp::LibraryDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.libraries.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::PipelineCreateCompute {
            payload,
            function_name,
        } => {
            let lib = shared
                .contexts
                .lock()
                .get(&ctx_id)
                .and_then(|c| c.libraries.get(&payload.library_id).cloned())
                .ok_or_else(|| GpuError::unknown_handle("unknown library_id"))?;
            let max_tg = (payload.max_total_threads_per_threadgroup > 0)
                .then_some(payload.max_total_threads_per_threadgroup);
            let pso = shared.platform.new_compute_pipeline(
                &lib,
                &function_name,
                max_tg,
                payload.threadgroup_size_multiple != 0,
            )?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .compute_pipelines
                .insert(payload.pipeline_id, pso);
            Ok(payload.pipeline_id)
        }
        ControlOp::PipelineCreateRender {
            payload,
            vertex_name,
            fragment_name,
        } => {
            let (vlib, flib) = {
                let contexts = shared.contexts.lock();
                let ctx = contexts
                    .get(&ctx_id)
                    .ok_or_else(|| GpuError::unknown_handle("unknown context"))?;
                let v = ctx
                    .libraries
                    .get(&payload.vertex_library_id)
                    .cloned()
                    .ok_or_else(|| GpuError::unknown_handle("unknown vertex_library_id"))?;
                let f = if payload.fragment_library_id != 0 {
                    Some(
                        ctx.libraries
                            .get(&payload.fragment_library_id)
                            .cloned()
                            .ok_or_else(|| {
                                GpuError::unknown_handle("unknown fragment_library_id")
                            })?,
                    )
                } else {
                    None
                };
                (v, f)
            };
            let mut color_attachments = [RenderColorAttachmentDesc::default(); 8];
            for (i, wire_att) in payload.color_attachments.iter().enumerate() {
                color_attachments[i] = RenderColorAttachmentDesc {
                    pixel_format: wire_att.pixel_format,
                    blending_enabled: wire_att.blending_enabled != 0,
                    src_rgb_factor: wire_att.src_rgb_factor,
                    dst_rgb_factor: wire_att.dst_rgb_factor,
                    rgb_blend_op: wire_att.rgb_blend_op,
                    src_alpha_factor: wire_att.src_alpha_factor,
                    dst_alpha_factor: wire_att.dst_alpha_factor,
                    alpha_blend_op: wire_att.alpha_blend_op,
                    write_mask: wire_att.write_mask,
                };
            }
            let rdesc = RenderPipelineDesc {
                depth_pixel_format: payload.depth_pixel_format,
                stencil_pixel_format: payload.stencil_pixel_format,
                sample_count: payload.sample_count.max(1),
                rasterization_enabled: payload.rasterization_enabled != 0,
                color_attachments,
            };
            let pso = shared.platform.new_render_pipeline(
                &vlib,
                &vertex_name,
                flib.as_ref(),
                fragment_name.as_deref(),
                &rdesc,
            )?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .render_pipelines
                .insert(payload.pipeline_id, pso);
            Ok(payload.pipeline_id)
        }
        ControlOp::PipelineDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.compute_pipelines.remove(&p.handle);
                ctx.render_pipelines.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::SurfaceImport(p) => {
            let existing = shared.surfaces.lock().get(&p.surface_id).cloned();
            let tex = if let Some(t) = existing {
                t
            } else {
                let desc = wire_to_tex_desc(&p.desc);
                let created = shared.platform.new_texture(&desc)?;
                if p.region_offset != u64::MAX {
                    let total = p.bytes_per_row.saturating_mul(u64::from(p.desc.height.max(1)));
                    let (off, len) = validate_region_slice(region_len, p.region_offset, total)?;
                    // SAFETY: Range validated inside `shared.region`.
                    let bytes = unsafe { copy_from_shared_raw(base.add(off), len) };
                    shared.platform.texture_replace_region(
                        &created,
                        [0, 0, 0],
                        [p.desc.width.max(1), p.desc.height.max(1), 1],
                        0,
                        0,
                        &bytes,
                        p.bytes_per_row,
                        total,
                    )?;
                }
                shared.surfaces.lock().insert(p.surface_id, created.clone());
                created
            };
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .textures
                .insert(p.texture_id, tex);
            Ok(p.texture_id)
        }
        ControlOp::SurfaceRelease(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.textures.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::DisplaySetMode(p) => {
            let (w, h) = normalize_mode(p.width, p.height);
            shared.mmio.display_width.store(w, Ordering::Release);
            shared.mmio.display_height.store(h, Ordering::Release);
            shared.frame_sink.set_mode(w, h);
            Ok(0)
        }
        ControlOp::DisplayPresent(p) => {
            let tex = get_texture(shared, ctx_id, p.texture_id)?;
            let (w, h) = shared.frame_sink.dimensions();
            let row_bytes = u64::from(w) * 4;
            let total = row_bytes * u64::from(h);
            let mut pixels = vec![0u8; total as usize];
            shared.platform.texture_get_bytes(
                &tex,
                [0, 0, 0],
                [w, h, 1],
                p.level,
                p.slice,
                &mut pixels,
                row_bytes,
                total,
            )?;
            shared.frame_sink.present_bgra8(w, h, &pixels);
            Ok(0)
        }
        ControlOp::EventCreate(p) => {
            let ev = shared.platform.new_event()?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .events
                .insert(p.handle, ev);
            Ok(p.handle)
        }
        ControlOp::EventDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.events.remove(&p.handle);
            }
            Ok(0)
        }
        ControlOp::FenceCreate(p) => {
            let f = shared.platform.new_fence()?;
            shared
                .contexts
                .lock()
                .entry(ctx_id)
                .or_default()
                .fences
                .insert(p.handle, f);
            Ok(p.handle)
        }
        ControlOp::FenceDestroy(p) => {
            if let Some(ctx) = shared.contexts.lock().get_mut(&ctx_id) {
                ctx.fences.remove(&p.handle);
            }
            Ok(0)
        }
    }
}

fn wire_to_tex_desc(w: &IvgTextureDescWire) -> TextureDesc {
    TextureDesc {
        texture_type: w.texture_type,
        pixel_format: w.pixel_format,
        width: w.width,
        height: w.height,
        depth: w.depth,
        mipmap_level_count: w.mipmap_level_count,
        sample_count: w.sample_count,
        array_length: w.array_length,
        usage: w.usage,
        storage_mode: w.storage_mode,
    }
}

fn get_buffer<P: Platform>(
    shared: &DeviceShared<P>,
    ctx_id: u32,
    id: u64,
) -> Result<P::Buffer, GpuError> {
    shared
        .contexts
        .lock()
        .get(&ctx_id)
        .and_then(|c| c.buffers.get(&id).cloned())
        .ok_or_else(|| GpuError::unknown_handle(format!("unknown buffer_id {id}")))
}

fn get_texture<P: Platform>(
    shared: &DeviceShared<P>,
    ctx_id: u32,
    id: u64,
) -> Result<P::Texture, GpuError> {
    shared
        .contexts
        .lock()
        .get(&ctx_id)
        .and_then(|c| c.textures.get(&id).cloned())
        .ok_or_else(|| GpuError::unknown_handle(format!("unknown texture_id {id}")))
}

fn queue_worker_loop<P: Platform>(
    shared: &DeviceShared<P>,
    ctx_id: u32,
    queue_id: u64,
    sub_ring: SharedRing,
    cmp_ring: SharedRing,
    wake: &EventFd,
    stop: &AtomicBool,
) {
    let mut scratch = Vec::with_capacity(4096);
    let mut recorded: Vec<QueueOp> = Vec::new();
    let mut in_cmd_buf = false;

    while !stop.load(Ordering::Acquire) && !shared.shutdown.load(Ordering::Acquire) {
        sub_ring.set_host_idle(false);
        let mut processed = false;

        while !stop.load(Ordering::Acquire) && !shared.shutdown.load(Ordering::Acquire) {
            match sub_ring.try_pop_record(&mut scratch) {
                Ok(Some(len)) => {
                    processed = true;
                    match decode_queue(&scratch[..len]) {
                        Ok((hdr, QueueOp::CmdBufferBegin(_))) => {
                            recorded.clear();
                            in_cmd_buf = true;
                            let _ = hdr;
                        }
                        Ok((hdr, QueueOp::CmdBufferCommit(commit))) => {
                            let ops = std::mem::take(&mut recorded);
                            in_cmd_buf = false;
                            let cmp = execute_command_buffer(shared, ctx_id, queue_id, hdr.seqno, &ops);
                            if commit.want_completion != 0 {
                                let _ = cmp_ring.push_bytes(cmp.as_bytes());
                                raise_irq_bit(shared, IVG_IRQ_QUEUE);
                            }
                        }
                        Ok((_, op)) => {
                            if in_cmd_buf {
                                recorded.push(op);
                            }
                        }
                        Err(e) => {
                            log::warn!("queue {queue_id} decode error: {e:?}");
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!("queue {queue_id} ring error: {e:?}");
                    break;
                }
            }
        }

        sub_ring.set_host_idle(true);
        if !processed {
            wait_eventfd_or_timeout(wake, 25);
        }
    }
}

fn execute_command_buffer<P: Platform>(
    shared: &DeviceShared<P>,
    ctx_id: u32,
    queue_id: u64,
    seqno: u64,
    ops: &[QueueOp],
) -> IvgCompletionRecord {
    match run_command_buffer_ops(shared, ctx_id, queue_id, ops) {
        Ok(res) => IvgCompletionRecord {
            seqno,
            status: res.status,
            refusal_code: res.refusal_code,
            gpu_start_ns: res.gpu_start_ns,
            gpu_end_ns: res.gpu_end_ns,
            result_u64: 0,
        },
        Err(err) => {
            shared.platform.ledger().record(&err);
            IvgCompletionRecord {
                seqno,
                status: err.metal_status,
                refusal_code: err.code as u32,
                gpu_start_ns: 0,
                gpu_end_ns: 0,
                result_u64: 0,
            }
        }
    }
}

fn run_command_buffer_ops<P: Platform>(
    shared: &DeviceShared<P>,
    ctx_id: u32,
    queue_id: u64,
    ops: &[QueueOp],
) -> Result<inferno_vgpu_vulkan::core::platform::CommandExecutionResult, GpuError> {
    let queue = shared
        .contexts
        .lock()
        .get(&ctx_id)
        .and_then(|c| c.queues.get(&queue_id).map(|(q, _)| q.clone()))
        .ok_or_else(|| GpuError::unknown_handle("unknown queue"))?;

    let mut cmd_buf = shared.platform.new_command_buffer(&queue)?;
    let mut idx = 0;

    while idx < ops.len() {
        match &ops[idx] {
            QueueOp::ComputeBegin => {
                idx += 1;
                let mut enc = shared.platform.begin_compute_encoder(&mut cmd_buf)?;
                while idx < ops.len() {
                    match &ops[idx] {
                        QueueOp::ComputeEnd => {
                            idx += 1;
                            break;
                        }
                        QueueOp::ComputeSetPipeline(p) => {
                            let pso = shared
                                .contexts
                                .lock()
                                .get(&ctx_id)
                                .and_then(|c| c.compute_pipelines.get(&p.pipeline_id).cloned())
                                .ok_or_else(|| GpuError::unknown_handle("unknown compute pipeline"))?;
                            enc.set_pipeline(&pso)?;
                        }
                        QueueOp::ComputeSetBuffer(p) => {
                            let buf = if p.buffer_id != 0 {
                                Some(get_buffer(shared, ctx_id, p.buffer_id)?)
                            } else {
                                None
                            };
                            enc.set_buffer(buf.as_ref(), p.offset, p.index)?;
                        }
                        QueueOp::ComputeSetTexture(p) => {
                            let tex = if p.texture_id != 0 {
                                Some(get_texture(shared, ctx_id, p.texture_id)?)
                            } else {
                                None
                            };
                            enc.set_texture(tex.as_ref(), p.index)?;
                        }
                        QueueOp::ComputeSetBytes { index, bytes } => {
                            enc.set_bytes(bytes, *index)?;
                        }
                        QueueOp::ComputeDispatchThreads(p) => {
                            enc.dispatch_threads(p.grid, p.threads_per_threadgroup)?;
                        }
                        QueueOp::ComputeDispatchThreadgroups(p) => {
                            enc.dispatch_threadgroups(p.grid, p.threads_per_threadgroup)?;
                        }
                        QueueOp::FenceUpdate(p) => {
                            let f = shared
                                .contexts
                                .lock()
                                .get(&ctx_id)
                                .and_then(|c| c.fences.get(&p.handle).cloned())
                                .ok_or_else(|| GpuError::unknown_handle("unknown fence"))?;
                            enc.update_fence(&f)?;
                        }
                        QueueOp::FenceWait(p) => {
                            let f = shared
                                .contexts
                                .lock()
                                .get(&ctx_id)
                                .and_then(|c| c.fences.get(&p.handle).cloned())
                                .ok_or_else(|| GpuError::unknown_handle("unknown fence"))?;
                            enc.wait_for_fence(&f)?;
                        }
                        _ => break,
                    }
                    idx += 1;
                }
                enc.end_encoding();
            }
            QueueOp::RenderBeginPass(p) => {
                idx += 1;
                let empty_att = || RenderPassColorAttachment {
                    texture: None,
                    clear_color: [0.0; 4],
                    level: 0,
                    slice: 0,
                    load_action: 0,
                    store_action: 0,
                };
                let mut atts = [
                    empty_att(),
                    empty_att(),
                    empty_att(),
                    empty_att(),
                    empty_att(),
                    empty_att(),
                    empty_att(),
                    empty_att(),
                ];
                for (i, w) in p.color_attachments.iter().enumerate() {
                    if w.texture_id != 0 {
                        let tex = get_texture(shared, ctx_id, w.texture_id)?;
                        atts[i] = RenderPassColorAttachment {
                            texture: Some(tex),
                            clear_color: w.clear_color,
                            level: w.level,
                            slice: w.slice,
                            load_action: w.load_action,
                            store_action: w.store_action,
                        };
                    }
                }
                let depth_texture = if p.depth_texture_id != 0 {
                    Some(get_texture(shared, ctx_id, p.depth_texture_id)?)
                } else {
                    None
                };
                let pass = RenderPassDesc {
                    render_target_width: p.render_target_width,
                    render_target_height: p.render_target_height,
                    default_raster_sample_count: p.default_raster_sample_count,
                    color_attachments: atts,
                    depth_texture,
                    clear_depth: p.clear_depth,
                    depth_load_action: p.depth_load_action,
                    depth_store_action: p.depth_store_action,
                };
                let mut enc = shared.platform.begin_render_encoder(&mut cmd_buf, &pass)?;
                while idx < ops.len() {
                    match &ops[idx] {
                        QueueOp::RenderEndPass => {
                            idx += 1;
                            break;
                        }
                        QueueOp::RenderSetPipeline(rp) => {
                            let pso = shared
                                .contexts
                                .lock()
                                .get(&ctx_id)
                                .and_then(|c| c.render_pipelines.get(&rp.pipeline_id).cloned())
                                .ok_or_else(|| GpuError::unknown_handle("unknown render pipeline"))?;
                            enc.set_pipeline(&pso)?;
                        }
                        QueueOp::RenderSetViewport(vp) => {
                            enc.set_viewport(Viewport {
                                origin_x: vp.origin_x,
                                origin_y: vp.origin_y,
                                width: vp.width,
                                height: vp.height,
                                znear: vp.znear,
                                zfar: vp.zfar,
                            })?;
                        }
                        QueueOp::RenderSetScissor(sc) => {
                            enc.set_scissor(ScissorRect {
                                x: sc.x,
                                y: sc.y,
                                width: sc.width,
                                height: sc.height,
                            })?;
                        }
                        QueueOp::RenderSetVertexBuffer(b) => {
                            let buf = if b.buffer_id != 0 {
                                Some(get_buffer(shared, ctx_id, b.buffer_id)?)
                            } else {
                                None
                            };
                            enc.set_vertex_buffer(buf.as_ref(), b.offset, b.index)?;
                        }
                        QueueOp::RenderSetFragmentBuffer(b) => {
                            let buf = if b.buffer_id != 0 {
                                Some(get_buffer(shared, ctx_id, b.buffer_id)?)
                            } else {
                                None
                            };
                            enc.set_fragment_buffer(buf.as_ref(), b.offset, b.index)?;
                        }
                        QueueOp::RenderSetFragmentTexture(t) => {
                            let tex = if t.texture_id != 0 {
                                Some(get_texture(shared, ctx_id, t.texture_id)?)
                            } else {
                                None
                            };
                            enc.set_fragment_texture(tex.as_ref(), t.index)?;
                        }
                        QueueOp::RenderSetVertexBytes { index, bytes } => {
                            enc.set_vertex_bytes(bytes, *index)?;
                        }
                        QueueOp::RenderSetFragmentBytes { index, bytes } => {
                            enc.set_fragment_bytes(bytes, *index)?;
                        }
                        QueueOp::RenderDrawPrimitives(d) => {
                            enc.draw_primitives(
                                d.primitive_type,
                                d.vertex_start,
                                d.vertex_count,
                                d.instance_count,
                                d.base_instance,
                            )?;
                        }
                        QueueOp::RenderDrawIndexed(d) => {
                            let ibuf = get_buffer(shared, ctx_id, d.index_buffer_id)?;
                            enc.draw_indexed_primitives(
                                d.primitive_type,
                                d.index_count,
                                d.index_type,
                                &ibuf,
                                d.index_buffer_offset,
                                d.instance_count,
                                d.base_vertex,
                                d.base_instance,
                            )?;
                        }
                        _ => break,
                    }
                    idx += 1;
                }
                enc.end_encoding();
            }
            QueueOp::BlitBegin => {
                idx += 1;
                let mut enc = shared.platform.begin_blit_encoder(&mut cmd_buf)?;
                while idx < ops.len() {
                    match &ops[idx] {
                        QueueOp::BlitEnd => {
                            idx += 1;
                            break;
                        }
                        QueueOp::BlitCopyBuffer(b) => {
                            let src = get_buffer(shared, ctx_id, b.src_buffer_id)?;
                            let dst = get_buffer(shared, ctx_id, b.dst_buffer_id)?;
                            enc.copy_buffer(&src, b.src_offset, &dst, b.dst_offset, b.size)?;
                        }
                        QueueOp::BlitCopyTexture(t) => {
                            let src = get_texture(shared, ctx_id, t.src_texture_id)?;
                            let dst = get_texture(shared, ctx_id, t.dst_texture_id)?;
                            enc.copy_texture(
                                &src,
                                t.src_slice,
                                t.src_level,
                                t.src_origin,
                                t.extent,
                                &dst,
                                t.dst_slice,
                                t.dst_level,
                                t.dst_origin,
                            )?;
                        }
                        QueueOp::BlitFillBuffer(b) => {
                            let buf = get_buffer(shared, ctx_id, b.buffer_id)?;
                            enc.fill_buffer(&buf, b.offset, b.length, b.value)?;
                        }
                        _ => break,
                    }
                    idx += 1;
                }
                enc.end_encoding();
            }
            QueueOp::EventSignal(p) => {
                let ev = shared
                    .contexts
                    .lock()
                    .get(&ctx_id)
                    .and_then(|c| c.events.get(&p.handle).cloned())
                    .ok_or_else(|| GpuError::unknown_handle("unknown event"))?;
                shared.platform.encode_signal_event(&mut cmd_buf, &ev, p.value)?;
                idx += 1;
            }
            QueueOp::EventWait(p) => {
                let ev = shared
                    .contexts
                    .lock()
                    .get(&ctx_id)
                    .and_then(|c| c.events.get(&p.handle).cloned())
                    .ok_or_else(|| GpuError::unknown_handle("unknown event"))?;
                shared.platform.encode_wait_event(&mut cmd_buf, &ev, p.value)?;
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    shared.platform.commit_and_wait(cmd_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inferno_vgpu_graphics::ops::encode_record;
    use inferno_vgpu_vulkan::core::testkit::FakePlatform;
    use std::time::Duration;
    use zerocopy::FromBytes;

    fn wait_completion(ring: &SharedRing) -> IvgCompletionRecord {
        let mut buf = [0u8; std::mem::size_of::<IvgCompletionRecord>()];
        for _ in 0..200 {
            if let Ok(true) = ring.try_pop_fixed(&mut buf) {
                return IvgCompletionRecord::read_from_bytes(&buf).expect("completion record");
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for completion");
    }

    #[test]
    fn device_end_to_end_with_fake_platform() {
        let irq_hits = Arc::new(AtomicU32::new(0));
        let irq_clone = Arc::clone(&irq_hits);
        let plat = Arc::new(FakePlatform::new());
        let dev = GraphicsDevice::new(
            plat,
            DeviceConfig {
                region_size: 4 * 1024 * 1024,
                display_width: 828,
                display_height: 1792,
                raise_irq: Arc::new(move |bits| {
                    irq_clone.fetch_or(bits, Ordering::SeqCst);
                }),
            },
        )
        .expect("create GraphicsDevice");

        assert_eq!(dev.mmio_read(regs::MAGIC), IVG_MAGIC);
        assert_eq!(dev.mmio_read(regs::VERSION), IVG_VERSION);

        let base = dev.region_ptr();
        let len = dev.region_size();
        // Control rings at 0x0 and 0x10000
        let ctrl_sub = unsafe { SharedRing::init(base, len, 0x0, 0x1000, 4096) }.expect("sub");
        let ctrl_cmp =
            unsafe { SharedRing::init(base, len, 0x10000, 0x11000, 4096) }.expect("cmp");

        dev.mmio_write(regs::CONTROL_SUB_OFF_LO, 0x0);
        dev.mmio_write(regs::CONTROL_CMP_OFF_LO, 0x10000);
        dev.mmio_write(regs::STATUS, IVG_STATUS_DRIVER_READY);

        // 1. Ping
        let ping = encode_record(control_op::PING, 1, 101, &IvgPingPayload { cookie: 0xCAFE });
        ctrl_sub.push_bytes(&ping).expect("push ping");
        dev.mmio_write(regs::CONTROL_DOORBELL, 1);

        let cmp = wait_completion(&ctrl_cmp);
        assert_eq!(cmp.seqno, 101);
        assert_eq!(cmp.status, 0);
        assert_eq!(cmp.result_u64, 0xCAFE);
        assert_ne!(irq_hits.load(Ordering::SeqCst) & IVG_IRQ_CONTROL, 0);

        // 2. Get device caps into Region offset 0x30000
        let get_caps = encode_record(
            control_op::GET_DEVICE_CAPS,
            1,
            102,
            &IvgGetCapsPayload {
                caps_region_offset: 0x30000,
            },
        );
        ctrl_sub.push_bytes(&get_caps).expect("push caps");
        dev.doorbell_write(0);
        let cmp = wait_completion(&ctrl_cmp);
        assert_eq!(cmp.status, 0);

        let caps_bytes = unsafe { copy_from_shared_raw(base.add(0x30000), std::mem::size_of::<IvgDeviceCaps>()) };
        let caps = IvgDeviceCaps::read_from_bytes(&caps_bytes).expect("caps");
        assert_eq!(caps.apple_gpu_family, 1006);
    }

    #[test]
    #[ignore = "requires host Vulkan GPU"]
    fn real_vulkan_device_wire_compute_and_render_e2e() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let rt = Arc::new(inferno_vgpu_vulkan::rt_platform::RtPlatform::new(None).expect("RtPlatform"));
        let dev = GraphicsDevice::new(rt, DeviceConfig::default()).expect("GraphicsDevice");
        let base = dev.region_ptr();
        let len = dev.region_size();

        let ctrl_sub = unsafe { SharedRing::init(base, len, 0x0, 0x1000, 16384) }.expect("csub");
        let ctrl_cmp =
            unsafe { SharedRing::init(base, len, 0x10000, 0x11000, 16384) }.expect("ccmp");
        let q_sub = unsafe { SharedRing::init(base, len, 0x20000, 0x21000, 16384) }.expect("qsub");
        let q_cmp = unsafe { SharedRing::init(base, len, 0x30000, 0x31000, 16384) }.expect("qcmp");

        dev.mmio_write(regs::CONTROL_SUB_OFF_LO, 0x0);
        dev.mmio_write(regs::CONTROL_CMP_OFF_LO, 0x10000);
        dev.mmio_write(regs::STATUS, IVG_STATUS_DRIVER_READY);

        let submit_ctrl = |bytes: &[u8]| -> IvgCompletionRecord {
            ctrl_sub.push_bytes(bytes).expect("push ctrl");
            dev.doorbell_write(0);
            wait_completion(&ctrl_cmp)
        };

        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::CONTEXT_CREATE,
                1,
                1,
                &IvgContextCreatePayload { pid: 100, reserved: 0 }
            ))
            .status,
            0
        );
        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::QUEUE_CREATE,
                1,
                2,
                &IvgQueueCreatePayload {
                    queue_id: 1,
                    sub_ring_offset: 0x20000,
                    cmp_ring_offset: 0x30000,
                    max_command_buffers: 8,
                    reserved: 0
                }
            ))
            .status,
            0
        );

        // Assemble runtime_examples.ll to AIR bitcode & upload to Region staging at 0x100000
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../vendor/metal2vulkan/crates/metal2vulkan-validation/fixtures/public/examples/runtime_examples.ll");
        let ir = std::fs::read_to_string(&fixture_path).expect("read fixture");
        let mut child = Command::new("llvm-as")
            .args(["-", "-o", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("llvm-as");
        child.stdin.as_mut().unwrap().write_all(ir.as_bytes()).unwrap();
        let bitcode = child.wait_with_output().unwrap().stdout;
        let sha: [u8; 32] = Sha256::digest(&bitcode).into();
        unsafe { copy_to_shared_raw(base.add(0x100000), &bitcode) };

        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::LIBRARY_CREATE,
                1,
                3,
                &IvgLibraryCreatePayload {
                    library_id: 10,
                    sha256: sha,
                    region_offset: 0x100000,
                    byte_len: bitcode.len() as u64,
                }
            ))
            .status,
            0
        );

        // Create compute pipeline (`scale_and_offset`) & render pipeline (`colored_vertex` + `colored_fragment`)
        let pso_payload = IvgComputePipelineCreatePayload {
            pipeline_id: 11,
            library_id: 10,
            function_name_len: 16,
            max_total_threads_per_threadgroup: 0,
            threadgroup_size_multiple: 0,
            reserved: 0,
        };
        assert_eq!(
            submit_ctrl(&inferno_vgpu_graphics::ops::encode_record_with_tail(
                control_op::PIPELINE_CREATE_COMPUTE,
                1,
                4,
                pso_payload.as_bytes(),
                b"scale_and_offset"
            ))
            .status,
            0
        );

        let mut rpso_payload = IvgRenderPipelineCreatePayload {
            pipeline_id: 12,
            vertex_library_id: 10,
            fragment_library_id: 10,
            vertex_name_len: 14,
            fragment_name_len: 16,
            depth_pixel_format: 0,
            stencil_pixel_format: 0,
            sample_count: 1,
            rasterization_enabled: 1,
            color_attachments: [IvgRenderColorAttachmentWire::default(); 8],
        };
        rpso_payload.color_attachments[0].pixel_format = 80; // BGRA8Unorm
        rpso_payload.color_attachments[0].write_mask = 0x0F;
        assert_eq!(
            submit_ctrl(&inferno_vgpu_graphics::ops::encode_record_with_tail(
                control_op::PIPELINE_CREATE_RENDER,
                1,
                5,
                rpso_payload.as_bytes(),
                b"colored_vertexcolored_fragment"
            ))
            .status,
            0
        );

        // Alloc block + buffers + upload inputs (`src`, `dst`, `scale`, `vpos`, `vcol`)
        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::BLOCK_ALLOC,
                1,
                6,
                &IvgBlockAllocPayload {
                    block_id: 1,
                    size: 4096,
                    memory_class: 0,
                    reserved: 0,
                }
            ))
            .status,
            0
        );
        for (bid, off) in [
            (101u64, 0u64),
            (102, 256),
            (103, 512),
            (104, 768),
            (105, 1024),
        ] {
            assert_eq!(
                submit_ctrl(&encode_record(
                    control_op::BUFFER_CREATE,
                    1,
                    10 + bid,
                    &IvgBufferCreatePayload {
                        buffer_id: bid,
                        block_id: 1,
                        block_offset: off,
                        length: 256,
                        gpu_address: 0,
                    }
                ))
                .status,
                0
            );
        }
        let src_bytes: Vec<u8> = (0..64).flat_map(|i| ((i as f32) + 1.0).to_le_bytes()).collect();
        let scale_bytes: Vec<u8> = 2.0f32.to_le_bytes().to_vec();
        let positions: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        let colors: [f32; 12] = [
            1.0, 0.25, 0.5, 1.0, 0.25, 1.0, 0.5, 1.0, 0.25, 0.5, 1.0, 1.0,
        ];
        let pos_bytes: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        let col_bytes: Vec<u8> = colors.iter().flat_map(|v| v.to_le_bytes()).collect();
        unsafe {
            copy_to_shared_raw(base.add(0x200000), &src_bytes);
            copy_to_shared_raw(base.add(0x201000), &scale_bytes);
            copy_to_shared_raw(base.add(0x202000), &pos_bytes);
            copy_to_shared_raw(base.add(0x203000), &col_bytes);
        }
        for (seq, bid, reg_off, len_b) in [
            (200u64, 101u64, 0x200000u64, 256u64),
            (201, 103, 0x201000, 4),
            (204, 104, 0x202000, pos_bytes.len() as u64),
            (205, 105, 0x203000, col_bytes.len() as u64),
        ] {
            assert_eq!(
                submit_ctrl(&encode_record(
                    control_op::BUFFER_UPLOAD,
                    1,
                    seq,
                    &IvgBufferTransferPayload {
                        buffer_id: bid,
                        buffer_offset: 0,
                        region_offset: reg_off,
                        length: len_b,
                    }
                ))
                .status,
                0
            );
        }

        // Dispatch compute over Queue 1
        q_sub
            .push_bytes(&encode_record(
                queue_op::CMD_BUFFER_BEGIN,
                1,
                1,
                &IvgCmdBufferBeginPayload { cmd_buf_id: 1 },
            ))
            .unwrap();
        q_sub
            .push_bytes(&inferno_vgpu_graphics::ops::encode_record_with_tail(
                queue_op::COMPUTE_BEGIN,
                1,
                2,
                &[],
                &[],
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::COMPUTE_SET_PIPELINE,
                1,
                3,
                &IvgComputeSetPipelinePayload { pipeline_id: 11 },
            ))
            .unwrap();
        for (slot, bid) in [(0u32, 101u64), (1, 102), (2, 103)] {
            q_sub
                .push_bytes(&encode_record(
                    queue_op::COMPUTE_SET_BUFFER,
                    1,
                    4 + u64::from(slot),
                    &IvgComputeSetBufferPayload {
                        buffer_id: bid,
                        offset: 0,
                        index: slot,
                        reserved: 0,
                    },
                ))
                .unwrap();
        }
        q_sub
            .push_bytes(&encode_record(
                queue_op::COMPUTE_DISPATCH_THREADS,
                1,
                7,
                &IvgComputeDispatchPayload {
                    grid: [64, 1, 1],
                    threads_per_threadgroup: [16, 1, 1],
                },
            ))
            .unwrap();
        q_sub
            .push_bytes(&inferno_vgpu_graphics::ops::encode_record_with_tail(
                queue_op::COMPUTE_END,
                1,
                8,
                &[],
                &[],
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::CMD_BUFFER_COMMIT,
                1,
                9,
                &IvgCmdBufferCommitPayload {
                    cmd_buf_id: 1,
                    want_completion: 1,
                    reserved: 0,
                },
            ))
            .unwrap();
        dev.doorbell_write(1);

        let qcmp = wait_completion(&q_cmp);
        assert_eq!(qcmp.seqno, 9);
        assert_eq!(qcmp.status, 0);

        // Readback compute output buffer
        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::BUFFER_READBACK,
                1,
                202,
                &IvgBufferTransferPayload {
                    buffer_id: 102,
                    buffer_offset: 0,
                    region_offset: 0x204000,
                    length: 256,
                }
            ))
            .status,
            0
        );
        let out_bytes = unsafe { copy_from_shared_raw(base.add(0x204000), 256) };
        for i in 0..64usize {
            let v = f32::from_le_bytes(out_bytes[i * 4..i * 4 + 4].try_into().unwrap());
            let expected = ((i as f32) + 1.0) * 2.0 + (i as f32);
            assert!(
                (v - expected).abs() < 1e-4,
                "wire compute [{i}]: got {v}, expected {expected}"
            );
        }

        // Create 64x64 BGRA8Unorm render target + draw triangle + readback + present
        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::TEXTURE_CREATE,
                1,
                210,
                &IvgTextureCreatePayload {
                    texture_id: 201,
                    desc: IvgTextureDescWire {
                        texture_type: 2,
                        pixel_format: 80,
                        width: 64,
                        height: 64,
                        depth: 1,
                        mipmap_level_count: 1,
                        sample_count: 1,
                        array_length: 1,
                        usage: 0x05,
                        storage_mode: 0,
                    },
                    gpu_resource_id: 0,
                }
            ))
            .status,
            0
        );

        let mut pass = IvgRenderBeginPassPayload {
            render_target_width: 64,
            render_target_height: 64,
            default_raster_sample_count: 1,
            reserved: 0,
            color_attachments: [IvgRenderPassColorAttachmentWire::default(); 8],
            depth_texture_id: 0,
            clear_depth: 1.0,
            depth_load_action: 0,
            depth_store_action: 0,
        };
        pass.color_attachments[0] = IvgRenderPassColorAttachmentWire {
            texture_id: 201,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            level: 0,
            slice: 0,
            load_action: 2,
            store_action: 1,
        };

        q_sub
            .push_bytes(&encode_record(
                queue_op::CMD_BUFFER_BEGIN,
                1,
                20,
                &IvgCmdBufferBeginPayload { cmd_buf_id: 2 },
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(queue_op::RENDER_BEGIN_PASS, 1, 21, &pass))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::RENDER_SET_PIPELINE,
                1,
                22,
                &IvgRenderSetPipelinePayload { pipeline_id: 12 },
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::RENDER_SET_VERTEX_BUFFER,
                1,
                23,
                &IvgRenderSetBufferPayload {
                    buffer_id: 104,
                    offset: 0,
                    index: 0,
                    reserved: 0,
                },
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::RENDER_SET_VERTEX_BUFFER,
                1,
                24,
                &IvgRenderSetBufferPayload {
                    buffer_id: 105,
                    offset: 0,
                    index: 1,
                    reserved: 0,
                },
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::RENDER_DRAW_PRIMITIVES,
                1,
                25,
                &IvgRenderDrawPrimitivesPayload {
                    primitive_type: 3,
                    vertex_start: 0,
                    vertex_count: 3,
                    instance_count: 1,
                    base_instance: 0,
                    reserved: 0,
                },
            ))
            .unwrap();
        q_sub
            .push_bytes(&inferno_vgpu_graphics::ops::encode_record_with_tail(
                queue_op::RENDER_END_PASS,
                1,
                26,
                &[],
                &[],
            ))
            .unwrap();
        q_sub
            .push_bytes(&encode_record(
                queue_op::CMD_BUFFER_COMMIT,
                1,
                27,
                &IvgCmdBufferCommitPayload {
                    cmd_buf_id: 2,
                    want_completion: 1,
                    reserved: 0,
                },
            ))
            .unwrap();
        dev.doorbell_write(1);

        let rcmp = wait_completion(&q_cmp);
        assert_eq!(rcmp.seqno, 27);
        assert_eq!(rcmp.status, 0);

        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::TEXTURE_READBACK,
                1,
                211,
                &IvgTextureTransferPayload {
                    texture_id: 201,
                    region_offset: 0x205000,
                    origin: [0, 0, 0],
                    extent: [64, 64, 1],
                    level: 0,
                    slice: 0,
                    bytes_per_row: 64 * 4,
                    bytes_per_image: 64 * 64 * 4,
                }
            ))
            .status,
            0
        );
        let px = unsafe { copy_from_shared_raw(base.add(0x205000), 64 * 64 * 4) };
        assert!(px.iter().any(|&b| b != 0 && b != 255));

        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::DISPLAY_SET_MODE,
                1,
                212,
                &IvgDisplaySetModePayload {
                    width: 64,
                    height: 64,
                    refresh_hz: 60,
                    reserved: 0,
                }
            ))
            .status,
            0
        );

        assert_eq!(
            submit_ctrl(&encode_record(
                control_op::DISPLAY_PRESENT,
                1,
                213,
                &IvgDisplayPresentPayload {
                    texture_id: 201,
                    level: 0,
                    slice: 0,
                }
            ))
            .status,
            0
        );
        let mut frame = vec![0u8; 64 * 64 * 4];
        let (fw, fh, fseq) = dev.read_display(&mut frame).expect("read_display");
        assert_eq!((fw, fh), (64, 64));
        assert!(fseq >= 1);
        assert!(frame.iter().any(|&b| b != 0 && b != 255));
    }
}
