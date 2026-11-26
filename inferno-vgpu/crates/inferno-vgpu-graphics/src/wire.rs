//! Binary wire protocol definitions shared between the guest iOS shim,
//! Inferno's `apple-pvgpu` device, and the host Rust GPU engine.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Magic value `"IVGP"` (`0x4956_4750`).
pub const IVG_MAGIC: u32 = 0x4956_4750;
/// Protocol major version.
pub const IVG_VERSION_MAJOR: u16 = 1;
/// Protocol minor version.
pub const IVG_VERSION_MINOR: u16 = 0;

/// Combined packed protocol version (`(major << 16) | minor`).
pub const IVG_VERSION: u32 = ((IVG_VERSION_MAJOR as u32) << 16) | (IVG_VERSION_MINOR as u32);

/// Host GPU has unified memory (`MTLDevice.hasUnifiedMemory`).
pub const IVG_FEAT_UNIFIED_MEMORY: u64 = 1 << 0;
/// Host exposes a shared host-cached memory pool.
pub const IVG_FEAT_SHARED_POOL: u64 = 1 << 1;
/// Host supports importing guest IOSurface backing pages.
pub const IVG_FEAT_SURFACE_PAGES: u64 = 1 << 2;
/// Device exposes per-context doorbell pages.
pub const IVG_FEAT_DOORBELL_PAGES: u64 = 1 << 3;
/// Host supports display scanout readback for VNC / `apple_displaypipe_v4`.
pub const IVG_FEAT_DISPLAY_READBACK: u64 = 1 << 4;

/// MMIO register offsets in Region 0.
pub mod regs {
    /// RO: `IVG_MAGIC`.
    pub const MAGIC: u64 = 0x000;
    /// RO: `IVG_VERSION`.
    pub const VERSION: u64 = 0x004;
    /// RO: Low 32 bits of `FEATURES`.
    pub const FEATURES_LO: u64 = 0x008;
    /// RO: High 32 bits of `FEATURES`.
    pub const FEATURES_HI: u64 = 0x00C;
    /// RW: Device status (`STATUS_*`).
    pub const STATUS: u64 = 0x010;
    /// RW: Low 32 bits of control submission ring header offset in Region 1.
    pub const CONTROL_SUB_OFF_LO: u64 = 0x018;
    /// RW: High 32 bits of control submission ring header offset in Region 1.
    pub const CONTROL_SUB_OFF_HI: u64 = 0x01C;
    /// WO: Wake control thread (Ring 0).
    pub const CONTROL_DOORBELL: u64 = 0x020;
    /// WO: Wake queue worker (`queue_id`).
    pub const QUEUE_DOORBELL: u64 = 0x024;
    /// RO: Interrupt status bitmask (`IRQ_*`).
    pub const IRQ_STATUS: u64 = 0x028;
    /// WO: Write bits to clear `IRQ_STATUS`.
    pub const IRQ_ACK: u64 = 0x02C;
    /// RO: Low 32 bits of Region 1 size in bytes.
    pub const REGION_SIZE_LO: u64 = 0x030;
    /// RO: High 32 bits of Region 1 size in bytes.
    pub const REGION_SIZE_HI: u64 = 0x034;
    /// RO: Low 32 bits of shared pool base (0 if none).
    pub const POOL_BASE_LO: u64 = 0x038;
    /// RO: High 32 bits of shared pool base.
    pub const POOL_BASE_HI: u64 = 0x03C;
    /// RO: Low 32 bits of shared pool size.
    pub const POOL_SIZE_LO: u64 = 0x040;
    /// RO: High 32 bits of shared pool size.
    pub const POOL_SIZE_HI: u64 = 0x044;
    /// RO: Configured display width in pixels.
    pub const DISPLAY_WIDTH: u64 = 0x048;
    /// RO: Configured display height in pixels.
    pub const DISPLAY_HEIGHT: u64 = 0x04C;
    /// RO: Last fatal / device-lost error code.
    pub const ERROR_CODE: u64 = 0x050;
    /// RW: Low 32 bits of control completion ring header offset in Region 1.
    pub const CONTROL_CMP_OFF_LO: u64 = 0x058;
    /// RW: High 32 bits of control completion ring header offset in Region 1.
    pub const CONTROL_CMP_OFF_HI: u64 = 0x05C;
}

/// Device status register bits.
pub const IVG_STATUS_RESET: u32 = 0;
/// Guest driver has initialized control rings.
pub const IVG_STATUS_DRIVER_READY: u32 = 1 << 0;
/// Host GPU engine is ready.
pub const IVG_STATUS_HOST_READY: u32 = 1 << 1;
/// Host connection was lost (device-lost recovery without SEP crash).
pub const IVG_STATUS_DEVICE_LOST: u32 = 1 << 31;

/// Interrupt status bits.
pub const IVG_IRQ_CONTROL: u32 = 1 << 0;
/// Command queue completion interrupt.
pub const IVG_IRQ_QUEUE: u32 = 1 << 1;
/// Device lost / fatal error interrupt.
pub const IVG_IRQ_ERROR: u32 = 1 << 31;

/// Ring header flag: host worker is currently asleep and needs a doorbell write.
pub const IVG_RING_FLAG_HOST_IDLE: u32 = 1 << 0;
/// Ring header flag: guest requests IRQ on completion.
pub const IVG_RING_FLAG_ARM_IRQ: u32 = 1 << 1;

/// Control ring opcodes (Ring 0).
pub mod control_op {
    /// Phase 0 ping-pong test.
    pub const PING: u16 = 0x0001;
    /// Query host `IvgDeviceCaps`.
    pub const GET_DEVICE_CAPS: u16 = 0x0002;
    /// Create a per-process context.
    pub const CONTEXT_CREATE: u16 = 0x0003;
    /// Destroy a context and all owned objects.
    pub const CONTEXT_DESTROY: u16 = 0x0004;
    /// Create a command queue with its submission/completion rings.
    pub const QUEUE_CREATE: u16 = 0x0005;
    /// Destroy a command queue.
    pub const QUEUE_DESTROY: u16 = 0x0006;
    /// Allocate a host memory block (`Shared` or `Private`).
    pub const BLOCK_ALLOC: u16 = 0x0007;
    /// Free a host memory block.
    pub const BLOCK_FREE: u16 = 0x0008;
    /// Create a buffer inside a block.
    pub const BUFFER_CREATE: u16 = 0x0009;
    /// Destroy a buffer.
    pub const BUFFER_DESTROY: u16 = 0x000A;
    /// Create a texture.
    pub const TEXTURE_CREATE: u16 = 0x000B;
    /// Destroy a texture.
    pub const TEXTURE_DESTROY: u16 = 0x000C;
    /// Upload bytes from Region 1 into a texture region.
    pub const TEXTURE_UPLOAD: u16 = 0x000D;
    /// Readback texture bytes into Region 1.
    pub const TEXTURE_READBACK: u16 = 0x000E;
    /// Create an AIR/metallib library.
    pub const LIBRARY_CREATE: u16 = 0x000F;
    /// Destroy a library.
    pub const LIBRARY_DESTROY: u16 = 0x0010;
    /// Create a compute pipeline state.
    pub const PIPELINE_CREATE_COMPUTE: u16 = 0x0011;
    /// Create a render pipeline state.
    pub const PIPELINE_CREATE_RENDER: u16 = 0x0012;
    /// Destroy a pipeline state.
    pub const PIPELINE_DESTROY: u16 = 0x0013;
    /// Import an IOSurface backing buffer/texture.
    pub const SURFACE_IMPORT: u16 = 0x0014;
    /// Release an imported IOSurface.
    pub const SURFACE_RELEASE: u16 = 0x0015;
    /// Configure display scanout dimensions.
    pub const DISPLAY_SET_MODE: u16 = 0x0016;
    /// Present a texture/surface to the emulator display.
    pub const DISPLAY_PRESENT: u16 = 0x0017;
    /// Synchronize buffer contents from Region 1 staging offset into host buffer.
    pub const BUFFER_UPLOAD: u16 = 0x0018;
    /// Read buffer contents from host buffer into Region 1 staging offset.
    pub const BUFFER_READBACK: u16 = 0x0019;
    /// Create an event/shared event.
    pub const EVENT_CREATE: u16 = 0x001A;
    /// Destroy an event.
    pub const EVENT_DESTROY: u16 = 0x001B;
    /// Create a fence.
    pub const FENCE_CREATE: u16 = 0x001C;
    /// Destroy a fence.
    pub const FENCE_DESTROY: u16 = 0x001D;
}

/// Queue submission ring opcodes (Ring 1..N).
pub mod queue_op {
    /// Start building a command buffer.
    pub const CMD_BUFFER_BEGIN: u16 = 0x0100;
    /// Commit a command buffer and emit completion record.
    pub const CMD_BUFFER_COMMIT: u16 = 0x0101;
    /// Begin a compute encoder.
    pub const COMPUTE_BEGIN: u16 = 0x0200;
    /// Set compute pipeline state.
    pub const COMPUTE_SET_PIPELINE: u16 = 0x0201;
    /// Bind compute buffer at slot.
    pub const COMPUTE_SET_BUFFER: u16 = 0x0202;
    /// Bind compute texture at slot.
    pub const COMPUTE_SET_TEXTURE: u16 = 0x0203;
    /// Bind inline bytes at slot (payload follows struct).
    pub const COMPUTE_SET_BYTES: u16 = 0x0204;
    /// Dispatch threads (`dispatchThreads:threadsPerThreadgroup:`).
    pub const COMPUTE_DISPATCH_THREADS: u16 = 0x0205;
    /// Dispatch threadgroups (`dispatchThreadgroups:threadsPerThreadgroup:`).
    pub const COMPUTE_DISPATCH_THREADGROUPS: u16 = 0x0206;
    /// End compute encoder.
    pub const COMPUTE_END: u16 = 0x0207;
    /// Begin render pass encoder.
    pub const RENDER_BEGIN_PASS: u16 = 0x0300;
    /// Set render pipeline state.
    pub const RENDER_SET_PIPELINE: u16 = 0x0301;
    /// Set viewport.
    pub const RENDER_SET_VIEWPORT: u16 = 0x0302;
    /// Set scissor rectangle.
    pub const RENDER_SET_SCISSOR: u16 = 0x0303;
    /// Set vertex buffer at slot.
    pub const RENDER_SET_VERTEX_BUFFER: u16 = 0x0304;
    /// Set fragment buffer at slot.
    pub const RENDER_SET_FRAGMENT_BUFFER: u16 = 0x0305;
    /// Set fragment texture at slot.
    pub const RENDER_SET_FRAGMENT_TEXTURE: u16 = 0x0306;
    /// Set vertex inline bytes (payload follows struct).
    pub const RENDER_SET_VERTEX_BYTES: u16 = 0x0307;
    /// Set fragment inline bytes (payload follows struct).
    pub const RENDER_SET_FRAGMENT_BYTES: u16 = 0x0308;
    /// Draw primitives.
    pub const RENDER_DRAW_PRIMITIVES: u16 = 0x0309;
    /// Draw indexed primitives.
    pub const RENDER_DRAW_INDEXED: u16 = 0x030A;
    /// End render encoder.
    pub const RENDER_END_PASS: u16 = 0x030B;
    /// Begin blit encoder.
    pub const BLIT_BEGIN: u16 = 0x0400;
    /// Copy buffer to buffer.
    pub const BLIT_COPY_BUFFER: u16 = 0x0401;
    /// Copy texture to texture.
    pub const BLIT_COPY_TEXTURE: u16 = 0x0402;
    /// Fill buffer with byte value.
    pub const BLIT_FILL_BUFFER: u16 = 0x0403;
    /// End blit encoder.
    pub const BLIT_END: u16 = 0x0404;
    /// Signal event.
    pub const EVENT_SIGNAL: u16 = 0x0500;
    /// Wait for event.
    pub const EVENT_WAIT: u16 = 0x0501;
    /// Update fence (inside encoder).
    pub const FENCE_UPDATE: u16 = 0x0502;
    /// Wait fence (inside encoder).
    pub const FENCE_WAIT: u16 = 0x0503;
}

/// Cache-line-aligned lockless SPSC ring header placed inside Region 1.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug, Default, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRingHeader {
    /// Producer byte cursor (monotonically wrapping `u32`).
    pub producer: u32,
    /// Consumer byte cursor (monotonically wrapping `u32`).
    pub consumer: u32,
    /// `(capacity_bytes - 1)`, where `capacity_bytes` is a power of two.
    pub capacity_mask: u32,
    /// `IVG_RING_FLAG_*` bitmask.
    pub flags: u32,
    /// Offset of ring data buffer relative to Region 1 base.
    pub data_offset: u64,
    /// Reserved for alignment to 64 bytes.
    pub reserved: [u64; 5],
}

/// Prefix of every record written onto a submission ring.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRecordHeader {
    /// Opcode (`control_op::*` or `queue_op::*`).
    pub opcode: u16,
    /// Record-specific flags.
    pub flags: u16,
    /// Total byte length of this record, including this header.
    pub size: u32,
    /// Context ID owning this command.
    pub context_id: u32,
    /// Reserved padding (8-byte alignment).
    pub reserved: u32,
    /// Monotonic sequence number for completion matching.
    pub seqno: u64,
}

/// Record written onto a completion ring when a control op or command buffer finishes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgCompletionRecord {
    /// Sequence number matching `IvgRecordHeader::seqno`.
    pub seqno: u64,
    /// Completion status (`0 = OK`, else `MTLCommandBufferError` or negative error code).
    pub status: u32,
    /// Structured refusal code from `metal2vulkan` / `AccuracyLedger` (`0 = none`).
    pub refusal_code: u32,
    /// GPU start timestamp in nanoseconds.
    pub gpu_start_ns: u64,
    /// GPU end timestamp in nanoseconds.
    pub gpu_end_ns: u64,
    /// Optional 64-bit return value (e.g., ping echo, caps offset).
    pub result_u64: u64,
}

/// Host device capabilities reported to guest via `OP_GET_DEVICE_CAPS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgDeviceCaps {
    /// Vulkan vendor ID.
    pub vendor_id: u32,
    /// Vulkan device ID.
    pub device_id: u32,
    /// `1` if `hasUnifiedMemory`, `0` otherwise.
    pub has_unified_memory: u32,
    /// Maximum threadgroup memory length in bytes.
    pub max_threadgroup_memory_length: u32,
    /// Maximum buffer length in bytes.
    pub max_buffer_length: u64,
    /// Read-write texture tier (`0`, `1`, `2`).
    pub read_write_texture_tier: u32,
    /// Argument buffers tier (`0`, `1`, `2`).
    pub argument_buffers_tier: u32,
    /// Reported Apple GPU family (e.g. `1005` = Apple5 / A12, `1006` = Apple6 / A13).
    pub apple_gpu_family: u32,
    /// `1` if function pointers are supported.
    pub supports_function_pointers: u32,
    /// UTF-8 null-terminated device name.
    pub device_name: [u8; 128],
}

impl Default for IvgDeviceCaps {
    fn default() -> Self {
        Self {
            vendor_id: 0,
            device_id: 0,
            has_unified_memory: 0,
            max_threadgroup_memory_length: 32768,
            max_buffer_length: 256 * 1024 * 1024,
            read_write_texture_tier: 2,
            argument_buffers_tier: 2,
            apple_gpu_family: 1006,
            supports_function_pointers: 1,
            device_name: [0u8; 128],
        }
    }
}

/// Payload for `OP_PING`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgPingPayload {
    /// Arbitrary cookie echoed in `IvgCompletionRecord::result_u64`.
    pub cookie: u64,
}

/// Payload for `OP_GET_DEVICE_CAPS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgGetCapsPayload {
    /// Region 1 offset where host writes `IvgDeviceCaps`.
    pub caps_region_offset: u64,
}

/// Payload for `OP_CONTEXT_CREATE` / `OP_CONTEXT_DESTROY`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgContextCreatePayload {
    /// Guest process PID (diagnostic only).
    pub pid: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `OP_QUEUE_CREATE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgQueueCreatePayload {
    /// Queue handle (64-bit).
    pub queue_id: u64,
    /// Offset of queue submission `IvgRingHeader` in Region 1.
    pub sub_ring_offset: u64,
    /// Offset of queue completion `IvgRingHeader` in Region 1.
    pub cmp_ring_offset: u64,
    /// Maximum command buffer count.
    pub max_command_buffers: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `OP_QUEUE_DESTROY` / generic 64-bit handle destroy ops.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgHandlePayload {
    /// 64-bit object handle.
    pub handle: u64,
}

/// Payload for `OP_BLOCK_ALLOC`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBlockAllocPayload {
    /// Guest-assigned block handle.
    pub block_id: u64,
    /// Size in bytes.
    pub size: u64,
    /// `0 = Shared`, `1 = Private`.
    pub memory_class: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `OP_BUFFER_CREATE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBufferCreatePayload {
    /// Guest-assigned buffer handle.
    pub buffer_id: u64,
    /// Parent block handle.
    pub block_id: u64,
    /// Byte offset inside the block.
    pub block_offset: u64,
    /// Buffer length in bytes.
    pub length: u64,
    /// Guest-pinned `gpuAddress` (0 = auto).
    pub gpu_address: u64,
}

/// Payload for `OP_BUFFER_UPLOAD` / `OP_BUFFER_READBACK`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBufferTransferPayload {
    /// Buffer handle.
    pub buffer_id: u64,
    /// Byte offset within buffer.
    pub buffer_offset: u64,
    /// Region 1 staging offset.
    pub region_offset: u64,
    /// Byte count.
    pub length: u64,
}

/// Wire description of `MTLTextureDescriptor`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgTextureDescWire {
    /// `MTLTextureType` raw value.
    pub texture_type: u32,
    /// `MTLPixelFormat` raw value.
    pub pixel_format: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Depth in texels.
    pub depth: u32,
    /// Mipmap level count.
    pub mipmap_level_count: u32,
    /// Sample count.
    pub sample_count: u32,
    /// Array length.
    pub array_length: u32,
    /// `MTLTextureUsage` raw bitmask.
    pub usage: u32,
    /// `MTLStorageMode` raw value (`0 = Shared`, `2 = Private`, `3 = Memoryless`).
    pub storage_mode: u32,
}

/// Payload for `OP_TEXTURE_CREATE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgTextureCreatePayload {
    /// Guest-assigned texture handle.
    pub texture_id: u64,
    /// Texture descriptor.
    pub desc: IvgTextureDescWire,
    /// Guest-pinned `gpuResourceID` (0 = auto).
    pub gpu_resource_id: u64,
}

/// Payload for `OP_TEXTURE_UPLOAD` / `OP_TEXTURE_READBACK`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgTextureTransferPayload {
    /// Texture handle.
    pub texture_id: u64,
    /// Region 1 staging offset.
    pub region_offset: u64,
    /// Region origin `[x, y, z]`.
    pub origin: [u32; 3],
    /// Region size `[w, h, d]`.
    pub extent: [u32; 3],
    /// Mipmap level.
    pub level: u32,
    /// Array slice.
    pub slice: u32,
    /// Bytes per row.
    pub bytes_per_row: u64,
    /// Bytes per image.
    pub bytes_per_image: u64,
}

/// Payload for `OP_LIBRARY_CREATE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgLibraryCreatePayload {
    /// Guest-assigned library handle.
    pub library_id: u64,
    /// SHA-256 of the AIR/metallib bytes.
    pub sha256: [u8; 32],
    /// Offset in Region 1 holding the AIR/metallib payload.
    pub region_offset: u64,
    /// Byte length of the AIR/metallib payload.
    pub byte_len: u64,
}

/// Payload for `OP_PIPELINE_CREATE_COMPUTE`.
/// Followed inline by `function_name_len` UTF-8 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgComputePipelineCreatePayload {
    /// Guest-assigned compute pipeline handle.
    pub pipeline_id: u64,
    /// Library handle.
    pub library_id: u64,
    /// Length in bytes of UTF-8 function name immediately following this struct.
    pub function_name_len: u32,
    /// `maxTotalThreadsPerThreadgroup` (0 = device default).
    pub max_total_threads_per_threadgroup: u32,
    /// `1` if `threadGroupSizeIsMultipleOfThreadExecutionWidth`.
    pub threadgroup_size_multiple: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Color attachment descriptor for render pipeline creation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderColorAttachmentWire {
    /// `MTLPixelFormat` raw value (`0` = unused).
    pub pixel_format: u32,
    /// `1` if blending enabled.
    pub blending_enabled: u8,
    /// `MTLBlendFactor` raw values.
    pub src_rgb_factor: u8,
    /// Destination RGB blend factor.
    pub dst_rgb_factor: u8,
    /// RGB blend operation.
    pub rgb_blend_op: u8,
    /// Source alpha blend factor.
    pub src_alpha_factor: u8,
    /// Destination alpha blend factor.
    pub dst_alpha_factor: u8,
    /// Alpha blend operation.
    pub alpha_blend_op: u8,
    /// `MTLColorWriteMask`.
    pub write_mask: u8,
}

/// Payload for `OP_PIPELINE_CREATE_RENDER`.
/// Followed inline by vertex function name (`vertex_name_len` bytes) then fragment function name (`fragment_name_len` bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderPipelineCreatePayload {
    /// Guest-assigned render pipeline handle.
    pub pipeline_id: u64,
    /// Vertex library handle.
    pub vertex_library_id: u64,
    /// Fragment library handle (`0` if no fragment function).
    pub fragment_library_id: u64,
    /// UTF-8 length of vertex function name.
    pub vertex_name_len: u32,
    /// UTF-8 length of fragment function name (`0` if none).
    pub fragment_name_len: u32,
    /// Depth attachment `MTLPixelFormat` (`0` = none).
    pub depth_pixel_format: u32,
    /// Stencil attachment `MTLPixelFormat` (`0` = none).
    pub stencil_pixel_format: u32,
    /// Raster sample count (`1` default).
    pub sample_count: u32,
    /// `1` if rasterization enabled, `0` if disabled.
    pub rasterization_enabled: u32,
    /// Color attachment descriptors (up to 8).
    pub color_attachments: [IvgRenderColorAttachmentWire; 8],
}

/// Payload for `OP_SURFACE_IMPORT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgSurfaceImportPayload {
    /// Guest-assigned texture handle representing this IOSurface.
    pub texture_id: u64,
    /// Global IOSurface ID.
    pub surface_id: u32,
    /// Plane index.
    pub plane: u32,
    /// Texture descriptor.
    pub desc: IvgTextureDescWire,
    /// Optional Region 1 backing offset (`u64::MAX` if managed internally).
    pub region_offset: u64,
    /// Bytes per row of backing.
    pub bytes_per_row: u64,
}

/// Payload for `OP_DISPLAY_SET_MODE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgDisplaySetModePayload {
    /// Display width in pixels.
    pub width: u32,
    /// Display height in pixels.
    pub height: u32,
    /// Refresh rate in Hz.
    pub refresh_hz: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `OP_DISPLAY_PRESENT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgDisplayPresentPayload {
    /// Texture handle to present (`BGRA8Unorm` / `RGBA8Unorm`).
    pub texture_id: u64,
    /// Mipmap level.
    pub level: u32,
    /// Array slice.
    pub slice: u32,
}

/// Payload for `CMD_BUFFER_BEGIN`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgCmdBufferBeginPayload {
    /// Guest-assigned command buffer handle.
    pub cmd_buf_id: u64,
}

/// Payload for `CMD_BUFFER_COMMIT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgCmdBufferCommitPayload {
    /// Command buffer handle.
    pub cmd_buf_id: u64,
    /// `1` if completion record should be emitted.
    pub want_completion: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `COMPUTE_SET_PIPELINE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgComputeSetPipelinePayload {
    /// Compute pipeline handle.
    pub pipeline_id: u64,
}

/// Payload for `COMPUTE_SET_BUFFER`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgComputeSetBufferPayload {
    /// Buffer handle (`0` = unbind / nil).
    pub buffer_id: u64,
    /// Byte offset inside buffer.
    pub offset: u64,
    /// Binding slot index.
    pub index: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `COMPUTE_SET_TEXTURE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgComputeSetTexturePayload {
    /// Texture handle (`0` = unbind / nil).
    pub texture_id: u64,
    /// Binding slot index.
    pub index: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `COMPUTE_SET_BYTES` / `RENDER_SET_VERTEX_BYTES` / `RENDER_SET_FRAGMENT_BYTES`.
/// Followed inline by `length` raw bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgSetBytesPayload {
    /// Binding slot index.
    pub index: u32,
    /// Byte length immediately following this struct.
    pub length: u32,
}

/// Payload for `COMPUTE_DISPATCH_THREADS` / `COMPUTE_DISPATCH_THREADGROUPS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgComputeDispatchPayload {
    /// Grid dimensions (`threads` or `threadgroups`).
    pub grid: [u32; 3],
    /// Threads per threadgroup.
    pub threads_per_threadgroup: [u32; 3],
}

/// Color attachment wire descriptor for `RENDER_BEGIN_PASS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderPassColorAttachmentWire {
    /// Target texture handle (`0` = unused).
    pub texture_id: u64,
    /// Clear color `[r, g, b, a]`.
    pub clear_color: [f64; 4],
    /// Mipmap level.
    pub level: u32,
    /// Array slice.
    pub slice: u32,
    /// `MTLLoadAction` (`0 = DontCare`, `1 = Load`, `2 = Clear`).
    pub load_action: u32,
    /// `MTLStoreAction` (`0 = DontCare`, `1 = Store`, ...).
    pub store_action: u32,
}

/// Payload for `RENDER_BEGIN_PASS`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderBeginPassPayload {
    /// Render target width (`0` = derive from attachments).
    pub render_target_width: u32,
    /// Render target height (`0` = derive from attachments).
    pub render_target_height: u32,
    /// Default raster sample count.
    pub default_raster_sample_count: u32,
    /// Reserved.
    pub reserved: u32,
    /// Color attachments (up to 8).
    pub color_attachments: [IvgRenderPassColorAttachmentWire; 8],
    /// Depth attachment texture (`0` = none).
    pub depth_texture_id: u64,
    /// Depth clear value.
    pub clear_depth: f64,
    /// Depth load action.
    pub depth_load_action: u32,
    /// Depth store action.
    pub depth_store_action: u32,
}

/// Payload for `RENDER_SET_PIPELINE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderSetPipelinePayload {
    /// Render pipeline handle.
    pub pipeline_id: u64,
}

/// Payload for `RENDER_SET_VIEWPORT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderSetViewportPayload {
    /// Origin X.
    pub origin_x: f64,
    /// Origin Y.
    pub origin_y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
    /// Near Z.
    pub znear: f64,
    /// Far Z.
    pub zfar: f64,
}

/// Payload for `RENDER_SET_SCISSOR`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderSetScissorPayload {
    /// X coordinate.
    pub x: u32,
    /// Y coordinate.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Payload for `RENDER_SET_VERTEX_BUFFER` / `RENDER_SET_FRAGMENT_BUFFER`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderSetBufferPayload {
    /// Buffer handle (`0` = nil).
    pub buffer_id: u64,
    /// Offset in buffer.
    pub offset: u64,
    /// Slot index.
    pub index: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `RENDER_SET_FRAGMENT_TEXTURE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderSetTexturePayload {
    /// Texture handle (`0` = nil).
    pub texture_id: u64,
    /// Slot index.
    pub index: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `RENDER_DRAW_PRIMITIVES`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderDrawPrimitivesPayload {
    /// `MTLPrimitiveType` raw value.
    pub primitive_type: u32,
    /// Starting vertex index.
    pub vertex_start: u32,
    /// Vertex count.
    pub vertex_count: u32,
    /// Instance count.
    pub instance_count: u32,
    /// Base instance.
    pub base_instance: u32,
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `RENDER_DRAW_INDEXED`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgRenderDrawIndexedPayload {
    /// `MTLPrimitiveType` raw value.
    pub primitive_type: u32,
    /// Index count.
    pub index_count: u32,
    /// `MTLIndexType` raw value (`0 = UInt16`, `1 = UInt32`).
    pub index_type: u32,
    /// Instance count.
    pub instance_count: u32,
    /// Index buffer handle.
    pub index_buffer_id: u64,
    /// Index buffer byte offset.
    pub index_buffer_offset: u64,
    /// Base vertex.
    pub base_vertex: i32,
    /// Base instance.
    pub base_instance: u32,
}

/// Payload for `BLIT_COPY_BUFFER`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBlitCopyBufferPayload {
    /// Source buffer handle.
    pub src_buffer_id: u64,
    /// Source offset.
    pub src_offset: u64,
    /// Destination buffer handle.
    pub dst_buffer_id: u64,
    /// Destination offset.
    pub dst_offset: u64,
    /// Byte length.
    pub size: u64,
}

/// Payload for `BLIT_COPY_TEXTURE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBlitCopyTexturePayload {
    /// Source texture handle.
    pub src_texture_id: u64,
    /// Source slice.
    pub src_slice: u32,
    /// Source level.
    pub src_level: u32,
    /// Source origin `[x, y, z]`.
    pub src_origin: [u32; 3],
    /// Copy extent `[w, h, d]`.
    pub extent: [u32; 3],
    /// Destination texture handle.
    pub dst_texture_id: u64,
    /// Destination slice.
    pub dst_slice: u32,
    /// Destination level.
    pub dst_level: u32,
    /// Destination origin `[x, y, z]`.
    pub dst_origin: [u32; 3],
    /// Reserved.
    pub reserved: u32,
}

/// Payload for `BLIT_FILL_BUFFER`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgBlitFillBufferPayload {
    /// Buffer handle.
    pub buffer_id: u64,
    /// Start offset.
    pub offset: u64,
    /// Fill length.
    pub length: u64,
    /// Fill byte value.
    pub value: u8,
    /// Padding.
    pub reserved: [u8; 7],
}

/// Payload for `EVENT_SIGNAL`, `EVENT_WAIT`, `FENCE_UPDATE`, `FENCE_WAIT`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgSyncPayload {
    /// Event or fence handle.
    pub handle: u64,
    /// Event value (`0` for fence).
    pub value: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn wire_struct_layouts_match_contract() {
        assert_eq!(size_of::<IvgRingHeader>(), 64);
        assert_eq!(align_of::<IvgRingHeader>(), 64);
        assert_eq!(offset_of!(IvgRingHeader, producer), 0);
        assert_eq!(offset_of!(IvgRingHeader, consumer), 4);
        assert_eq!(offset_of!(IvgRingHeader, capacity_mask), 8);
        assert_eq!(offset_of!(IvgRingHeader, flags), 12);
        assert_eq!(offset_of!(IvgRingHeader, data_offset), 16);

        assert_eq!(size_of::<IvgRecordHeader>(), 24);
        assert_eq!(size_of::<IvgCompletionRecord>(), 40);
        assert_eq!(size_of::<IvgDeviceCaps>(), 168);
        assert_eq!(size_of::<IvgTextureDescWire>(), 40);
        assert_eq!(size_of::<IvgRenderBeginPassPayload>(), 488);
    }
}
