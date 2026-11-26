//! `Platform` trait abstracting GPU execution across `RtPlatform` (real Vulkan via
//! `metal2vulkan-runtime`) and `FakePlatform` (deterministic unit tests).

use crate::core::caps::DeviceCaps;
use crate::core::refusal::{AccuracyLedger, GpuError};

/// Memory allocation class (`Shared` host-visible vs `Private` device-local).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryClass {
    /// Host-cached / shared memory.
    Shared,
    /// Device-local VRAM.
    Private,
}

/// Texture creation descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureDesc {
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
    /// `MTLStorageMode` raw value.
    pub storage_mode: u32,
}

/// Color attachment descriptor for render pipeline creation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderColorAttachmentDesc {
    /// `MTLPixelFormat` raw value (`0` = unused).
    pub pixel_format: u32,
    /// Blending enabled.
    pub blending_enabled: bool,
    /// Source RGB blend factor.
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
    /// Color write mask.
    pub write_mask: u8,
}

/// Render pipeline descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderPipelineDesc {
    /// Depth pixel format (`0` = none).
    pub depth_pixel_format: u32,
    /// Stencil pixel format (`0` = none).
    pub stencil_pixel_format: u32,
    /// Raster sample count.
    pub sample_count: u32,
    /// Rasterization enabled.
    pub rasterization_enabled: bool,
    /// Color attachments.
    pub color_attachments: [RenderColorAttachmentDesc; 8],
}

/// Color attachment in a render pass.
#[derive(Clone, Debug)]
pub struct RenderPassColorAttachment<T> {
    /// Target texture (`None` = unused).
    pub texture: Option<T>,
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

/// Render pass descriptor.
#[derive(Clone, Debug)]
pub struct RenderPassDesc<T> {
    /// Render target width (`0` = auto).
    pub render_target_width: u32,
    /// Render target height (`0` = auto).
    pub render_target_height: u32,
    /// Default raster sample count.
    pub default_raster_sample_count: u32,
    /// Color attachments.
    pub color_attachments: [RenderPassColorAttachment<T>; 8],
    /// Optional depth texture.
    pub depth_texture: Option<T>,
    /// Depth clear value.
    pub clear_depth: f64,
    /// Depth load action.
    pub depth_load_action: u32,
    /// Depth store action.
    pub depth_store_action: u32,
}

/// Viewport rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
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

/// Scissor rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScissorRect {
    /// X coordinate.
    pub x: u32,
    /// Y coordinate.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Completed command buffer execution summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandExecutionResult {
    /// `0 = ok`, else Metal command buffer error code.
    pub status: u32,
    /// Refusal code if an internal refusal occurred.
    pub refusal_code: u32,
    /// GPU start timestamp in nanoseconds.
    pub gpu_start_ns: u64,
    /// GPU end timestamp in nanoseconds.
    pub gpu_end_ns: u64,
}

/// Compute command encoder abstraction.
pub trait ComputeEncoderOps<P: Platform + ?Sized> {
    /// Sets compute pipeline state.
    fn set_pipeline(&mut self, pipeline: &P::ComputePipeline) -> Result<(), GpuError>;
    /// Sets buffer at slot.
    fn set_buffer(
        &mut self,
        buffer: Option<&P::Buffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError>;
    /// Sets texture at slot.
    fn set_texture(&mut self, texture: Option<&P::Texture>, index: u32) -> Result<(), GpuError>;
    /// Sets inline bytes at slot.
    fn set_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError>;
    /// Dispatches exact thread grid.
    fn dispatch_threads(&mut self, grid: [u32; 3], tg: [u32; 3]) -> Result<(), GpuError>;
    /// Dispatches threadgroups.
    fn dispatch_threadgroups(&mut self, groups: [u32; 3], tg: [u32; 3]) -> Result<(), GpuError>;
    /// Updates fence.
    fn update_fence(&mut self, fence: &P::Fence) -> Result<(), GpuError>;
    /// Waits for fence.
    fn wait_for_fence(&mut self, fence: &P::Fence) -> Result<(), GpuError>;
    /// Ends encoding.
    fn end_encoding(self: Box<Self>);
}

/// Render command encoder abstraction.
pub trait RenderEncoderOps<P: Platform + ?Sized> {
    /// Sets render pipeline state.
    fn set_pipeline(&mut self, pipeline: &P::RenderPipeline) -> Result<(), GpuError>;
    /// Sets viewport.
    fn set_viewport(&mut self, viewport: Viewport) -> Result<(), GpuError>;
    /// Sets scissor rect.
    fn set_scissor(&mut self, scissor: ScissorRect) -> Result<(), GpuError>;
    /// Sets vertex buffer.
    fn set_vertex_buffer(
        &mut self,
        buffer: Option<&P::Buffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError>;
    /// Sets fragment buffer.
    fn set_fragment_buffer(
        &mut self,
        buffer: Option<&P::Buffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError>;
    /// Sets fragment texture.
    fn set_fragment_texture(
        &mut self,
        texture: Option<&P::Texture>,
        index: u32,
    ) -> Result<(), GpuError>;
    /// Sets vertex inline bytes.
    fn set_vertex_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError>;
    /// Sets fragment inline bytes.
    fn set_fragment_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError>;
    /// Draws primitives.
    fn draw_primitives(
        &mut self,
        primitive_type: u32,
        vertex_start: u32,
        vertex_count: u32,
        instance_count: u32,
        base_instance: u32,
    ) -> Result<(), GpuError>;
    /// Draws indexed primitives.
    #[allow(clippy::too_many_arguments)]
    fn draw_indexed_primitives(
        &mut self,
        primitive_type: u32,
        index_count: u32,
        index_type: u32,
        index_buffer: &P::Buffer,
        index_buffer_offset: u64,
        instance_count: u32,
        base_vertex: i32,
        base_instance: u32,
    ) -> Result<(), GpuError>;
    /// Ends encoding.
    fn end_encoding(self: Box<Self>);
}

/// Blit command encoder abstraction.
pub trait BlitEncoderOps<P: Platform + ?Sized> {
    /// Copies buffer to buffer.
    fn copy_buffer(
        &mut self,
        src: &P::Buffer,
        src_offset: u64,
        dst: &P::Buffer,
        dst_offset: u64,
        size: u64,
    ) -> Result<(), GpuError>;
    /// Copies texture to texture.
    #[allow(clippy::too_many_arguments)]
    fn copy_texture(
        &mut self,
        src: &P::Texture,
        src_slice: u32,
        src_level: u32,
        src_origin: [u32; 3],
        extent: [u32; 3],
        dst: &P::Texture,
        dst_slice: u32,
        dst_level: u32,
        dst_origin: [u32; 3],
    ) -> Result<(), GpuError>;
    /// Fills buffer range with byte value.
    fn fill_buffer(
        &mut self,
        buffer: &P::Buffer,
        offset: u64,
        length: u64,
        value: u8,
    ) -> Result<(), GpuError>;
    /// Ends encoding.
    fn end_encoding(self: Box<Self>);
}

/// Backend GPU platform trait implemented by `RtPlatform` and `FakePlatform`.
pub trait Platform: Send + Sync + 'static {
    /// Memory block handle.
    type Block: Clone + Send + Sync + 'static;
    /// Buffer handle.
    type Buffer: Clone + Send + Sync + 'static;
    /// Texture handle.
    type Texture: Clone + Send + Sync + 'static;
    /// Shader library handle.
    type Library: Clone + Send + Sync + 'static;
    /// Compute pipeline state handle.
    type ComputePipeline: Clone + Send + Sync + 'static;
    /// Render pipeline state handle.
    type RenderPipeline: Clone + Send + Sync + 'static;
    /// Command queue handle.
    type Queue: Clone + Send + Sync + 'static;
    /// Command buffer handle.
    type CommandBuffer: Send + Sync + 'static;
    /// Event handle.
    type Event: Clone + Send + Sync + 'static;
    /// Fence handle.
    type Fence: Clone + Send + Sync + 'static;

    /// Returns device capabilities.
    fn caps(&self) -> &DeviceCaps;
    /// Returns the accuracy ledger for this platform.
    fn ledger(&self) -> &AccuracyLedger;
    /// Allocates a memory block.
    fn alloc_block(&self, class: MemoryClass, size: u64) -> Result<Self::Block, GpuError>;
    /// Creates a buffer inside a block.
    fn new_buffer(
        &self,
        block: &Self::Block,
        offset: u64,
        length: u64,
    ) -> Result<Self::Buffer, GpuError>;
    /// Writes bytes to a Shared buffer.
    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8])
    -> Result<(), GpuError>;
    /// Reads bytes from a Shared buffer.
    fn read_buffer(
        &self,
        buffer: &Self::Buffer,
        offset: u64,
        out: &mut [u8],
    ) -> Result<(), GpuError>;
    /// Creates a texture.
    fn new_texture(&self, desc: &TextureDesc) -> Result<Self::Texture, GpuError>;
    /// Uploads bytes into a texture region.
    #[allow(clippy::too_many_arguments)]
    fn texture_replace_region(
        &self,
        texture: &Self::Texture,
        origin: [u32; 3],
        extent: [u32; 3],
        level: u32,
        slice: u32,
        bytes: &[u8],
        bytes_per_row: u64,
        bytes_per_image: u64,
    ) -> Result<(), GpuError>;
    /// Reads back bytes from a texture region.
    #[allow(clippy::too_many_arguments)]
    fn texture_get_bytes(
        &self,
        texture: &Self::Texture,
        origin: [u32; 3],
        extent: [u32; 3],
        level: u32,
        slice: u32,
        out: &mut [u8],
        bytes_per_row: u64,
        bytes_per_image: u64,
    ) -> Result<(), GpuError>;
    /// Parses a metallib or AIR bitcode library.
    fn new_library(&self, bytes: &[u8]) -> Result<Self::Library, GpuError>;
    /// Compiles a compute pipeline state.
    fn new_compute_pipeline(
        &self,
        library: &Self::Library,
        function_name: &str,
        max_total_threads: Option<u32>,
        threadgroup_size_multiple: bool,
    ) -> Result<Self::ComputePipeline, GpuError>;
    /// Compiles a render pipeline state.
    fn new_render_pipeline(
        &self,
        vertex_lib: &Self::Library,
        vertex_name: &str,
        fragment_lib: Option<&Self::Library>,
        fragment_name: Option<&str>,
        desc: &RenderPipelineDesc,
    ) -> Result<Self::RenderPipeline, GpuError>;
    /// Creates a command queue.
    fn new_command_queue(&self, max_command_buffers: u32) -> Result<Self::Queue, GpuError>;
    /// Creates a command buffer from a queue.
    fn new_command_buffer(&self, queue: &Self::Queue) -> Result<Self::CommandBuffer, GpuError>;
    /// Begins a compute command encoder.
    fn begin_compute_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn ComputeEncoderOps<Self> + 'a>, GpuError>;
    /// Begins a render command encoder.
    fn begin_render_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
        pass: &RenderPassDesc<Self::Texture>,
    ) -> Result<Box<dyn RenderEncoderOps<Self> + 'a>, GpuError>;
    /// Begins a blit command encoder.
    fn begin_blit_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn BlitEncoderOps<Self> + 'a>, GpuError>;
    /// Signals an event on the command buffer.
    fn encode_signal_event(
        &self,
        cmd_buf: &mut Self::CommandBuffer,
        event: &Self::Event,
        value: u64,
    ) -> Result<(), GpuError>;
    /// Waits for an event on the command buffer.
    fn encode_wait_event(
        &self,
        cmd_buf: &mut Self::CommandBuffer,
        event: &Self::Event,
        value: u64,
    ) -> Result<(), GpuError>;
    /// Commits a command buffer and waits for completion.
    fn commit_and_wait(
        &self,
        cmd_buf: Self::CommandBuffer,
    ) -> Result<CommandExecutionResult, GpuError>;
    /// Creates a new event.
    fn new_event(&self) -> Result<Self::Event, GpuError>;
    /// Creates a new fence.
    fn new_fence(&self) -> Result<Self::Fence, GpuError>;
    /// Quiesces the platform safely prior to teardown/exit.
    fn quiesce(&self);
}
