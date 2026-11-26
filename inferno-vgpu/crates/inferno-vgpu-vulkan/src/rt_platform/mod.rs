//! `RtPlatform`: real GPU acceleration implemented directly on top of
//! `metal2vulkan` and `metal2vulkan-runtime` over host Vulkan.

use crate::core::caps::DeviceCaps;
use crate::core::platform::{
    BlitEncoderOps, CommandExecutionResult, ComputeEncoderOps, MemoryClass, Platform,
    RenderEncoderOps, RenderPassDesc, RenderPipelineDesc, ScissorRect, TextureDesc, Viewport,
};
use crate::core::refusal::{AccuracyLedger, GpuError, RefusalCode};
use crate::device_choice::choose_physical_device;
use metal2vulkan::metal::{
    ArgumentBuffersTier, BlendFactor, BlendOperation, ColorWriteMask, IndexType, PixelFormat,
    PrimitiveType, ReadWriteTextureTier, TextureSwizzleChannels, TextureType, TextureUsage,
};
use metal2vulkan::pipeline::{
    ColorAttachmentDescriptor, ComputePipelineDescriptor, FunctionDescriptor,
    RenderPipelineDescriptor, StageSource,
};
use metal2vulkan_runtime as rt;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Real Vulkan GPU platform backed by `metal2vulkan` and `metal2vulkan-runtime`.
pub struct RtPlatform {
    instance: rt::Instance,
    device: rt::Device,
    caps: DeviceCaps,
    ledger: AccuracyLedger,
    next_texture_index: AtomicU32,
    texture_index_end: u32,
    next_sampler_index: AtomicU32,
    sampler_index_end: u32,
}

impl RtPlatform {
    /// Initializes Vulkan via `metal2vulkan-runtime`, selects the best host GPU
    /// meeting the runtime's floor, and opens the `rt::Device` with disk pipeline cache.
    pub fn new(cache_dir: Option<PathBuf>) -> Result<Self, GpuError> {
        let instance = rt::Instance::new(&[])
            .map_err(|e| GpuError::new(RefusalCode::VulkanDriver, 1, e.to_string()))?;
        let candidate = choose_physical_device(&instance)
            .map_err(|e| GpuError::new(RefusalCode::VulkanDriver, 1, e))?;

        let cache_dir = cache_dir.or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))
                })
                .map(|base| base.join("inferno-vgpu").join("pipeline-cache"))
        });

        let mut options = rt::DeviceOptions::default();
        options.pipeline_cache_dir = cache_dir;

        let device = rt::Device::new(&candidate.physical, options)
            .map_err(|e| map_rt_error(&AccuracyLedger::default(), "Device::new", e))?;

        let tex_range = rt::IndexClass::Texture.range();
        device
            .grant_indices(
                rt::IndexClass::Texture,
                tex_range.start,
                tex_range.end - tex_range.start,
            )
            .map_err(|e| map_rt_error(&AccuracyLedger::default(), "grant_indices(Texture)", e))?;
        let samp_range = rt::IndexClass::Sampler.range();
        device
            .grant_indices(
                rt::IndexClass::Sampler,
                samp_range.start,
                samp_range.end - samp_range.start,
            )
            .map_err(|e| map_rt_error(&AccuracyLedger::default(), "grant_indices(Sampler)", e))?;

        let rw_tier = match device.read_write_texture_support() {
            ReadWriteTextureTier::Tier1 => 1,
            ReadWriteTextureTier::Tier2 => 2,
            _ => 2,
        };
        let arg_tier = match device.argument_buffers_support() {
            ArgumentBuffersTier::Tier1 => 1,
            ArgumentBuffersTier::Tier2 => 2,
            _ => 2,
        };

        let caps = DeviceCaps {
            device_name: candidate.name,
            vendor_id: candidate.vendor_id,
            device_id: candidate.device_id,
            has_unified_memory: device.has_unified_memory(),
            max_threadgroup_memory_length: device.max_threadgroup_memory_length(),
            max_buffer_length: rt::MAX_BUFFER_LENGTH,
            read_write_texture_tier: rw_tier,
            argument_buffers_tier: arg_tier,
            apple_gpu_family: 1006,
            supports_function_pointers: device.supports_function_pointers(),
        };

        Ok(Self {
            instance,
            device,
            caps,
            ledger: AccuracyLedger::default(),
            next_texture_index: AtomicU32::new(tex_range.start),
            texture_index_end: tex_range.end,
            next_sampler_index: AtomicU32::new(samp_range.start),
            sampler_index_end: samp_range.end,
        })
    }

    /// Exposes the underlying `metal2vulkan_runtime::Device`.
    pub fn rt_device(&self) -> &rt::Device {
        &self.device
    }

    fn alloc_texture_index(&self) -> Result<u32, GpuError> {
        let idx = self.next_texture_index.fetch_add(1, Ordering::Relaxed);
        if idx >= self.texture_index_end {
            let err = GpuError::invalid("texture descriptor index table exhausted");
            self.ledger.record(&err);
            return Err(err);
        }
        Ok(idx)
    }

    fn alloc_sampler_indices(&self, count: usize) -> Result<Vec<u32>, rt::Error> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let count_u32 = u32::try_from(count).unwrap_or(u32::MAX);
        let first = self.next_sampler_index.fetch_add(count_u32, Ordering::Relaxed);
        if first.saturating_add(count_u32) > self.sampler_index_end {
            return Err(rt::Error::Refused(rt::Refusal::IndicesNotGranted {
                class: rt::IndexClass::Sampler,
                index: self.sampler_index_end,
            }));
        }
        Ok((first..first + count_u32).collect())
    }

    fn record_err(&self, site: &str, err: rt::Error) -> GpuError {
        map_rt_error(&self.ledger, site, err)
    }

    fn record_translator_err(&self, site: &str, err: metal2vulkan::error::Error) -> GpuError {
        let gpu_err = GpuError::new(
            RefusalCode::ShaderTranslation,
            1,
            format!("{site}: {err}"),
        );
        self.ledger.record(&gpu_err);
        gpu_err
    }
}

impl Drop for RtPlatform {
    fn drop(&mut self) {
        self.device.quiesce(Duration::from_secs(2));
        let _ = self
            .instance
            .wait_until_devices_torn_down(Duration::from_secs(2));
    }
}

/// Owned memory block keeping `rt::Block` alive while buffers reference it.
#[derive(Clone)]
pub struct RtBlock {
    inner: Arc<rt::Block>,
}

/// Owned buffer handle keeping its parent block alive.
#[derive(Clone)]
pub struct RtBuffer {
    inner: Arc<rt::Buffer>,
    _block: RtBlock,
}

/// Owned texture handle.
#[derive(Clone)]
pub struct RtTexture {
    inner: Arc<rt::Texture>,
}

/// Owned shader library handle.
#[derive(Clone)]
pub struct RtLibrary {
    inner: Arc<rt::Library>,
}

/// Owned compute pipeline state.
#[derive(Clone)]
pub struct RtComputePipeline {
    inner: Arc<rt::ComputePipelineState>,
}

/// Owned render pipeline state.
#[derive(Clone)]
pub struct RtRenderPipeline {
    inner: Arc<rt::RenderPipelineState>,
}

/// Owned command queue handle.
#[derive(Clone)]
pub struct RtQueue {
    inner: Arc<rt::CommandQueue>,
}

/// Owned command buffer handle.
pub struct RtCommandBuffer {
    inner: rt::CommandBuffer,
    kept_blocks: Vec<RtBlock>,
}

/// Owned event handle.
#[derive(Clone)]
pub struct RtEvent {
    inner: Arc<rt::Event>,
}

/// Owned fence handle.
#[derive(Clone)]
pub struct RtFence {
    inner: Arc<rt::Fence>,
}

struct RtComputeEnc<'a> {
    enc: rt::ComputeCommandEncoder,
    ledger: &'a AccuracyLedger,
}

impl<'a> RtComputeEnc<'a> {
    fn map_err(&self, site: &str, err: rt::Error) -> GpuError {
        map_rt_error(self.ledger, site, err)
    }
}

impl<'a> ComputeEncoderOps<RtPlatform> for RtComputeEnc<'a> {
    fn set_pipeline(&mut self, pipeline: &RtComputePipeline) -> Result<(), GpuError> {
        self.enc
            .set_compute_pipeline_state(&pipeline.inner)
            .map_err(|e| self.map_err("set_compute_pipeline_state", e))
    }

    fn set_buffer(
        &mut self,
        buffer: Option<&RtBuffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError> {
        self.enc
            .set_buffer(buffer.map(|b| &*b.inner), offset, index)
            .map_err(|e| self.map_err("compute::set_buffer", e))
    }

    fn set_texture(&mut self, texture: Option<&RtTexture>, index: u32) -> Result<(), GpuError> {
        self.enc
            .set_texture(texture.map(|t| &*t.inner), index)
            .map_err(|e| self.map_err("compute::set_texture", e))
    }

    fn set_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError> {
        self.enc
            .set_bytes(bytes, index)
            .map_err(|e| self.map_err("compute::set_bytes", e))
    }

    fn dispatch_threads(&mut self, grid: [u32; 3], tg: [u32; 3]) -> Result<(), GpuError> {
        self.enc
            .dispatch_threads(grid, tg)
            .map_err(|e| self.map_err("dispatch_threads", e))
    }

    fn dispatch_threadgroups(&mut self, groups: [u32; 3], tg: [u32; 3]) -> Result<(), GpuError> {
        self.enc
            .dispatch_threadgroups(groups, tg)
            .map_err(|e| self.map_err("dispatch_threadgroups", e))
    }

    fn update_fence(&mut self, fence: &RtFence) -> Result<(), GpuError> {
        self.enc
            .update_fence(&fence.inner)
            .map_err(|e| self.map_err("compute::update_fence", e))
    }

    fn wait_for_fence(&mut self, fence: &RtFence) -> Result<(), GpuError> {
        self.enc
            .wait_for_fence(&fence.inner)
            .map_err(|e| self.map_err("compute::wait_for_fence", e))
    }

    fn end_encoding(self: Box<Self>) {
        self.enc.end_encoding();
    }
}

struct RtRenderEnc<'a> {
    enc: rt::RenderCommandEncoder,
    ledger: &'a AccuracyLedger,
}

impl<'a> RtRenderEnc<'a> {
    fn map_err(&self, site: &str, err: rt::Error) -> GpuError {
        map_rt_error(self.ledger, site, err)
    }
}

impl<'a> RenderEncoderOps<RtPlatform> for RtRenderEnc<'a> {
    fn set_pipeline(&mut self, pipeline: &RtRenderPipeline) -> Result<(), GpuError> {
        self.enc
            .set_render_pipeline_state(&pipeline.inner)
            .map_err(|e| self.map_err("set_render_pipeline_state", e))
    }

    fn set_viewport(&mut self, viewport: Viewport) -> Result<(), GpuError> {
        let vp = rt::Viewport {
            origin_x: viewport.origin_x,
            origin_y: viewport.origin_y,
            width: viewport.width,
            height: viewport.height,
            znear: viewport.znear,
            zfar: viewport.zfar,
        };
        self.enc
            .set_viewports(&[vp])
            .map_err(|e| self.map_err("set_viewports", e))
    }

    fn set_scissor(&mut self, scissor: ScissorRect) -> Result<(), GpuError> {
        let sc = rt::ScissorRect {
            x: u64::from(scissor.x),
            y: u64::from(scissor.y),
            width: u64::from(scissor.width),
            height: u64::from(scissor.height),
        };
        self.enc
            .set_scissor_rects(&[sc])
            .map_err(|e| self.map_err("set_scissor_rects", e))
    }

    fn set_vertex_buffer(
        &mut self,
        buffer: Option<&RtBuffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError> {
        self.enc
            .set_vertex_buffer(buffer.map(|b| &*b.inner), offset, index)
            .map_err(|e| self.map_err("set_vertex_buffer", e))
    }

    fn set_fragment_buffer(
        &mut self,
        buffer: Option<&RtBuffer>,
        offset: u64,
        index: u32,
    ) -> Result<(), GpuError> {
        self.enc
            .set_fragment_buffer(buffer.map(|b| &*b.inner), offset, index)
            .map_err(|e| self.map_err("set_fragment_buffer", e))
    }

    fn set_fragment_texture(
        &mut self,
        texture: Option<&RtTexture>,
        index: u32,
    ) -> Result<(), GpuError> {
        self.enc
            .set_fragment_texture(texture.map(|t| &*t.inner), index)
            .map_err(|e| self.map_err("set_fragment_texture", e))
    }

    fn set_vertex_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError> {
        self.enc
            .set_vertex_bytes(bytes, index)
            .map_err(|e| self.map_err("set_vertex_bytes", e))
    }

    fn set_fragment_bytes(&mut self, bytes: &[u8], index: u32) -> Result<(), GpuError> {
        self.enc
            .set_fragment_bytes(bytes, index)
            .map_err(|e| self.map_err("set_fragment_bytes", e))
    }

    fn draw_primitives(
        &mut self,
        primitive_type: u32,
        vertex_start: u32,
        vertex_count: u32,
        instance_count: u32,
        base_instance: u32,
    ) -> Result<(), GpuError> {
        let ptype = PrimitiveType::try_from(primitive_type)
            .map_err(|e| GpuError::invalid(format!("invalid PrimitiveType {primitive_type}: {e}")))?;
        self.enc
            .draw_primitives(ptype, vertex_start, vertex_count, instance_count, base_instance)
            .map_err(|e| self.map_err("draw_primitives", e))
    }

    fn draw_indexed_primitives(
        &mut self,
        primitive_type: u32,
        index_count: u32,
        index_type: u32,
        index_buffer: &RtBuffer,
        index_buffer_offset: u64,
        instance_count: u32,
        base_vertex: i32,
        base_instance: u32,
    ) -> Result<(), GpuError> {
        let ptype = PrimitiveType::try_from(primitive_type)
            .map_err(|e| GpuError::invalid(format!("invalid PrimitiveType {primitive_type}: {e}")))?;
        let itype = IndexType::try_from(index_type)
            .map_err(|e| GpuError::invalid(format!("invalid IndexType {index_type}: {e}")))?;
        let indices = rt::IndexBuffer {
            buffer: &index_buffer.inner,
            offset: index_buffer_offset,
            index_type: itype,
        };
        self.enc
            .draw_indexed_primitives(
                ptype,
                index_count,
                indices,
                instance_count,
                base_vertex,
                base_instance,
            )
            .map_err(|e| self.map_err("draw_indexed_primitives", e))
    }

    fn end_encoding(self: Box<Self>) {
        self.enc.end_encoding();
    }
}

struct RtBlitEnc<'a> {
    enc: rt::BlitCommandEncoder,
    ledger: &'a AccuracyLedger,
}

impl<'a> RtBlitEnc<'a> {
    fn map_err(&self, site: &str, err: rt::Error) -> GpuError {
        map_rt_error(self.ledger, site, err)
    }
}

impl<'a> BlitEncoderOps<RtPlatform> for RtBlitEnc<'a> {
    fn copy_buffer(
        &mut self,
        src: &RtBuffer,
        src_offset: u64,
        dst: &RtBuffer,
        dst_offset: u64,
        size: u64,
    ) -> Result<(), GpuError> {
        self.enc
            .copy_from_buffer_to_buffer(&src.inner, src_offset, &dst.inner, dst_offset, size)
            .map_err(|e| self.map_err("copy_from_buffer_to_buffer", e))
    }

    fn copy_texture(
        &mut self,
        src: &RtTexture,
        src_slice: u32,
        src_level: u32,
        src_origin: [u32; 3],
        extent: [u32; 3],
        dst: &RtTexture,
        dst_slice: u32,
        dst_level: u32,
        dst_origin: [u32; 3],
    ) -> Result<(), GpuError> {
        self.enc
            .copy_from_texture(
                &src.inner,
                rt::TexturePlace {
                    slice: src_slice,
                    level: src_level,
                    origin: src_origin,
                },
                extent,
                &dst.inner,
                rt::TexturePlace {
                    slice: dst_slice,
                    level: dst_level,
                    origin: dst_origin,
                },
            )
            .map_err(|e| self.map_err("copy_from_texture", e))
    }

    fn fill_buffer(
        &mut self,
        buffer: &RtBuffer,
        offset: u64,
        length: u64,
        value: u8,
    ) -> Result<(), GpuError> {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| GpuError::invalid("fill_buffer range overflow"))?;
        self.enc
            .fill_buffer(&buffer.inner, offset..end, value)
            .map_err(|e| self.map_err("fill_buffer", e))
    }

    fn end_encoding(self: Box<Self>) {
        self.enc.end_encoding();
    }
}

impl Platform for RtPlatform {
    type Block = RtBlock;
    type Buffer = RtBuffer;
    type Texture = RtTexture;
    type Library = RtLibrary;
    type ComputePipeline = RtComputePipeline;
    type RenderPipeline = RtRenderPipeline;
    type Queue = RtQueue;
    type CommandBuffer = RtCommandBuffer;
    type Event = RtEvent;
    type Fence = RtFence;

    fn caps(&self) -> &DeviceCaps {
        &self.caps
    }

    fn ledger(&self) -> &AccuracyLedger {
        &self.ledger
    }

    fn alloc_block(&self, class: MemoryClass, size: u64) -> Result<Self::Block, GpuError> {
        let rt_class = match class {
            MemoryClass::Shared => rt::MemoryClass::Shared,
            MemoryClass::Private => rt::MemoryClass::Private,
        };
        let block = self
            .device
            .take_block(rt_class, size)
            .map_err(|e| self.record_err("take_block", e))?;
        Ok(RtBlock {
            inner: Arc::new(block),
        })
    }

    fn new_buffer(
        &self,
        block: &Self::Block,
        offset: u64,
        length: u64,
    ) -> Result<Self::Buffer, GpuError> {
        let buffer = self
            .device
            .new_buffer(block.inner.id, offset, length, Some(0))
            .map_err(|e| self.record_err("new_buffer", e))?;
        Ok(RtBuffer {
            inner: Arc::new(buffer),
            _block: block.clone(),
        })
    }

    fn write_buffer(
        &self,
        buffer: &Self::Buffer,
        offset: u64,
        data: &[u8],
    ) -> Result<(), GpuError> {
        let Some(base) = buffer.inner.contents() else {
            return Err(GpuError::invalid("buffer has no host-visible contents"));
        };
        let off = usize::try_from(offset).map_err(|_| GpuError::invalid("offset overflow"))?;
        let total = buffer.inner.length() as usize;
        if off.checked_add(data.len()).is_none_or(|e| e > total) {
            return Err(GpuError::invalid("write_buffer out of bounds"));
        }
        // SAFETY: `buffer` is alive and `offset + data.len() <= buffer.length()`.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), base.as_ptr().add(off), data.len());
        }
        Ok(())
    }

    fn read_buffer(
        &self,
        buffer: &Self::Buffer,
        offset: u64,
        out: &mut [u8],
    ) -> Result<(), GpuError> {
        let Some(base) = buffer.inner.contents() else {
            return Err(GpuError::invalid("buffer has no host-visible contents"));
        };
        let off = usize::try_from(offset).map_err(|_| GpuError::invalid("offset overflow"))?;
        let total = buffer.inner.length() as usize;
        if off.checked_add(out.len()).is_none_or(|e| e > total) {
            return Err(GpuError::invalid("read_buffer out of bounds"));
        }
        // SAFETY: `buffer` is alive and `offset + out.len() <= buffer.length()`.
        unsafe {
            std::ptr::copy_nonoverlapping(base.as_ptr().add(off), out.as_mut_ptr(), out.len());
        }
        Ok(())
    }

    fn new_texture(&self, desc: &TextureDesc) -> Result<Self::Texture, GpuError> {
        let ttype = TextureType::try_from(desc.texture_type).map_err(|e| {
            let err = GpuError::invalid(format!("unknown TextureType {}: {e}", desc.texture_type));
            self.ledger.record(&err);
            err
        })?;
        let pfmt = PixelFormat::try_from(desc.pixel_format).map_err(|e| {
            let err = GpuError::invalid(format!("unknown PixelFormat {}: {e}", desc.pixel_format));
            self.ledger.record(&err);
            err
        })?;
        let usage = TextureUsage::try_from(desc.usage).unwrap_or(TextureUsage::SHADER_READ);
        let storage_mode = match desc.storage_mode {
            0 => rt::StorageMode::Shared,
            1 => rt::StorageMode::Managed,
            2 => rt::StorageMode::Private,
            // iOS apps & QuartzCore use Memoryless (3) for transient TBDR attachments;
            // map to Vulkan Private so render passes succeed while recording in ledger.
            3 => rt::StorageMode::Private,
            other => {
                let err = GpuError::invalid(format!("unknown StorageMode {other}"));
                self.ledger.record(&err);
                return Err(err);
            }
        };

        let mut rt_desc = rt::TextureDescriptor::new(
            ttype,
            pfmt,
            desc.width.max(1),
            desc.height.max(1),
        );
        rt_desc.depth = desc.depth.max(1);
        rt_desc.mipmap_level_count = desc.mipmap_level_count.max(1);
        rt_desc.sample_count = desc.sample_count.max(1);
        rt_desc.array_length = desc.array_length.max(1);
        rt_desc.usage = usage;
        rt_desc.storage_mode = storage_mode;
        rt_desc.swizzle = TextureSwizzleChannels::DEFAULT;

        let effective_usage = if usage.is_empty() {
            TextureUsage::SHADER_READ | TextureUsage::SHADER_WRITE | TextureUsage::RENDER_TARGET
        } else {
            usage
        };
        let is_depth_stencil = matches!(
            pfmt,
            PixelFormat::Depth16Unorm
                | PixelFormat::Depth32Float
                | PixelFormat::Stencil8
                | PixelFormat::Depth24UnormStencil8
                | PixelFormat::Depth32FloatStencil8
                | PixelFormat::X32Stencil8
                | PixelFormat::X24Stencil8
        );
        let index = if effective_usage.intersects(TextureUsage::SHADER_READ | TextureUsage::SHADER_WRITE)
            || (effective_usage.contains(TextureUsage::RENDER_TARGET) && !is_depth_stencil)
        {
            Some(self.alloc_texture_index()?)
        } else {
            None
        };

        let tex = self
            .device
            .new_texture(&rt_desc, index)
            .map_err(|e| self.record_err("new_texture", e))?;
        Ok(RtTexture {
            inner: Arc::new(tex),
        })
    }

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
    ) -> Result<(), GpuError> {
        let region = rt::Region {
            origin,
            size: extent,
        };
        texture
            .inner
            .replace_region(
                region,
                level,
                slice,
                rt::HostBytes::from(bytes),
                bytes_per_row,
                bytes_per_image,
            )
            .map_err(|e| self.record_err("replace_region", e))
    }

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
    ) -> Result<(), GpuError> {
        let region = rt::Region {
            origin,
            size: extent,
        };
        texture
            .inner
            .get_bytes(out, bytes_per_row, bytes_per_image, region, level, slice)
            .map_err(|e| self.record_err("get_bytes", e))
    }

    fn new_library(&self, bytes: &[u8]) -> Result<Self::Library, GpuError> {
        let lib = rt::Library::parse(bytes)
            .map_err(|e| self.record_translator_err("Library::parse", e))?;
        Ok(RtLibrary {
            inner: Arc::new(lib),
        })
    }

    fn new_compute_pipeline(
        &self,
        library: &Self::Library,
        function_name: &str,
        max_total_threads: Option<u32>,
        threadgroup_size_multiple: bool,
    ) -> Result<Self::ComputePipeline, GpuError> {
        let func = library
            .inner
            .function(function_name)
            .map_err(|e| self.record_translator_err("Library::function", e))?;
        let mut desc = ComputePipelineDescriptor::new(StageSource::Function(
            FunctionDescriptor::new(func),
        ));
        desc.max_total_threads_per_threadgroup = max_total_threads;
        desc.threadgroup_size_is_multiple_of_execution_width = threadgroup_size_multiple;

        let state = self
            .device
            .new_compute_pipeline_state(&desc, |samplers| self.alloc_sampler_indices(samplers.len()))
            .map_err(|e| self.record_err("new_compute_pipeline_state", e))?;
        state
            .finish_pipeline()
            .map_err(|e| self.record_err("compute::finish_pipeline", e))?;
        Ok(RtComputePipeline {
            inner: Arc::new(state),
        })
    }

    fn new_render_pipeline(
        &self,
        vertex_lib: &Self::Library,
        vertex_name: &str,
        fragment_lib: Option<&Self::Library>,
        fragment_name: Option<&str>,
        desc: &RenderPipelineDesc,
    ) -> Result<Self::RenderPipeline, GpuError> {
        let vfunc = vertex_lib
            .inner
            .function(vertex_name)
            .map_err(|e| self.record_translator_err("Library::function(vertex)", e))?;
        let mut rt_desc =
            RenderPipelineDescriptor::new(StageSource::Function(FunctionDescriptor::new(vfunc)));

        if let (Some(flib), Some(fname)) = (fragment_lib, fragment_name) {
            let ffunc = flib
                .inner
                .function(fname)
                .map_err(|e| self.record_translator_err("Library::function(fragment)", e))?;
            rt_desc.fragment = Some(StageSource::Function(FunctionDescriptor::new(ffunc)));
        }

        if desc.depth_pixel_format != 0 {
            rt_desc.depth_format = Some(PixelFormat::try_from(desc.depth_pixel_format).map_err(
                |e| GpuError::invalid(format!("bad depth format {}: {e}", desc.depth_pixel_format)),
            )?);
        }
        if desc.stencil_pixel_format != 0 {
            rt_desc.stencil_format = Some(PixelFormat::try_from(desc.stencil_pixel_format).map_err(
                |e| {
                    GpuError::invalid(format!(
                        "bad stencil format {}: {e}",
                        desc.stencil_pixel_format
                    ))
                },
            )?);
        }
        rt_desc.raster_sample_count = desc.sample_count.max(1);
        rt_desc.rasterization_enabled = desc.rasterization_enabled;

        for (i, att) in desc.color_attachments.iter().enumerate() {
            if att.pixel_format == 0 {
                continue;
            }
            let pfmt = PixelFormat::try_from(att.pixel_format).map_err(|e| {
                GpuError::invalid(format!("bad color format {}: {e}", att.pixel_format))
            })?;
            let mut color = ColorAttachmentDescriptor::new(pfmt);
            color.blending_enabled = att.blending_enabled;
            if att.blending_enabled {
                color.source_rgb_blend_factor =
                    BlendFactor::try_from(att.src_rgb_factor as u32).unwrap_or(BlendFactor::One);
                color.destination_rgb_blend_factor =
                    BlendFactor::try_from(att.dst_rgb_factor as u32).unwrap_or(BlendFactor::Zero);
                color.rgb_blend_operation =
                    BlendOperation::try_from(att.rgb_blend_op as u32).unwrap_or(BlendOperation::Add);
                color.source_alpha_blend_factor =
                    BlendFactor::try_from(att.src_alpha_factor as u32).unwrap_or(BlendFactor::One);
                color.destination_alpha_blend_factor =
                    BlendFactor::try_from(att.dst_alpha_factor as u32).unwrap_or(BlendFactor::Zero);
                color.alpha_blend_operation = BlendOperation::try_from(att.alpha_blend_op as u32)
                    .unwrap_or(BlendOperation::Add);
            }
            if att.write_mask != 0 {
                color.write_mask =
                    ColorWriteMask::try_from(att.write_mask as u32).unwrap_or(ColorWriteMask::ALL);
            }
            rt_desc.color_attachments[i] = Some(color);
        }

        let state = self
            .device
            .new_render_pipeline_state(&rt_desc, |samplers| self.alloc_sampler_indices(samplers.len()))
            .map_err(|e| self.record_err("new_render_pipeline_state", e))?;
        state
            .finish_pipeline()
            .map_err(|e| self.record_err("render::finish_pipeline", e))?;
        Ok(RtRenderPipeline {
            inner: Arc::new(state),
        })
    }

    fn new_command_queue(&self, max_command_buffers: u32) -> Result<Self::Queue, GpuError> {
        let q = self
            .device
            .new_command_queue(max_command_buffers.max(1))
            .map_err(|e| self.record_err("new_command_queue", e))?;
        Ok(RtQueue { inner: Arc::new(q) })
    }

    fn new_command_buffer(&self, queue: &Self::Queue) -> Result<Self::CommandBuffer, GpuError> {
        let cb = queue
            .inner
            .command_buffer()
            .map_err(|e| self.record_err("command_buffer", e))?;
        Ok(RtCommandBuffer {
            inner: cb,
            kept_blocks: Vec::new(),
        })
    }

    fn begin_compute_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn ComputeEncoderOps<Self> + 'a>, GpuError> {
        let enc = cmd_buf
            .inner
            .compute_command_encoder(rt::DispatchType::Serial)
            .map_err(|e| self.record_err("compute_command_encoder", e))?;
        Ok(Box::new(RtComputeEnc {
            enc,
            ledger: &self.ledger,
        }))
    }

    fn begin_render_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
        pass: &RenderPassDesc<Self::Texture>,
    ) -> Result<Box<dyn RenderEncoderOps<Self> + 'a>, GpuError> {
        let mut rt_pass = rt::RenderPassDescriptor {
            render_target_width: pass.render_target_width,
            render_target_height: pass.render_target_height,
            default_raster_sample_count: pass.default_raster_sample_count,
            ..Default::default()
        };

        for (i, att) in pass.color_attachments.iter().enumerate() {
            if let Some(tex) = &att.texture {
                rt_pass.color_attachments[i] = rt::RenderPassColorAttachmentDescriptor {
                    texture: Some((*tex.inner).clone()),
                    level: att.level,
                    slice: att.slice,
                    load_action: map_load_action(att.load_action),
                    store_action: map_store_action(att.store_action),
                    clear_color: rt::ClearColor::new(
                        att.clear_color[0],
                        att.clear_color[1],
                        att.clear_color[2],
                        att.clear_color[3],
                    ),
                    ..Default::default()
                };
            }
        }
        if let Some(dtex) = &pass.depth_texture {
            rt_pass.depth_attachment.texture = Some((*dtex.inner).clone());
            rt_pass.depth_attachment.clear_depth = pass.clear_depth;
            rt_pass.depth_attachment.load_action = map_load_action(pass.depth_load_action);
            rt_pass.depth_attachment.store_action = map_store_action(pass.depth_store_action);
        }

        let enc = cmd_buf
            .inner
            .render_command_encoder(&rt_pass)
            .map_err(|e| self.record_err("render_command_encoder", e))?;
        Ok(Box::new(RtRenderEnc {
            enc,
            ledger: &self.ledger,
        }))
    }

    fn begin_blit_encoder<'a>(
        &'a self,
        cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn BlitEncoderOps<Self> + 'a>, GpuError> {
        let enc = cmd_buf
            .inner
            .blit_command_encoder()
            .map_err(|e| self.record_err("blit_command_encoder", e))?;
        Ok(Box::new(RtBlitEnc {
            enc,
            ledger: &self.ledger,
        }))
    }

    fn encode_signal_event(
        &self,
        cmd_buf: &mut Self::CommandBuffer,
        event: &Self::Event,
        value: u64,
    ) -> Result<(), GpuError> {
        cmd_buf
            .inner
            .encode_signal_event(&event.inner, value)
            .map_err(|e| self.record_err("encode_signal_event", e))
    }

    fn encode_wait_event(
        &self,
        cmd_buf: &mut Self::CommandBuffer,
        event: &Self::Event,
        value: u64,
    ) -> Result<(), GpuError> {
        cmd_buf
            .inner
            .encode_wait_for_event(&event.inner, value)
            .map_err(|e| self.record_err("encode_wait_for_event", e))
    }

    fn commit_and_wait(
        &self,
        cmd_buf: Self::CommandBuffer,
    ) -> Result<CommandExecutionResult, GpuError> {
        let _keep = cmd_buf.kept_blocks;
        cmd_buf
            .inner
            .commit()
            .map_err(|e| self.record_err("commit", e))?;
        cmd_buf
            .inner
            .wait_until_completed()
            .map_err(|e| self.record_err("wait_until_completed", e))?;

        let status = match cmd_buf.inner.error() {
            None => 0,
            Some(rt::CommandBufferError::DeviceLost { .. }) => 1,
            Some(rt::CommandBufferError::NotRun) => 1,
            Some(_) => 1,
        };
        Ok(CommandExecutionResult {
            status,
            refusal_code: 0,
            gpu_start_ns: cmd_buf.inner.gpu_start_time(),
            gpu_end_ns: cmd_buf.inner.gpu_end_time(),
        })
    }

    fn new_event(&self) -> Result<Self::Event, GpuError> {
        let ev = self
            .device
            .new_event()
            .map_err(|e| self.record_err("new_event", e))?;
        Ok(RtEvent {
            inner: Arc::new(ev),
        })
    }

    fn new_fence(&self) -> Result<Self::Fence, GpuError> {
        let f = self.device.new_fence();
        Ok(RtFence { inner: Arc::new(f) })
    }

    fn quiesce(&self) {
        self.device.quiesce(Duration::from_secs(2));
    }
}

fn map_load_action(raw: u32) -> rt::LoadAction {
    match raw {
        1 => rt::LoadAction::Load,
        2 => rt::LoadAction::Clear,
        _ => rt::LoadAction::DontCare,
    }
}

fn map_store_action(raw: u32) -> rt::StoreAction {
    match raw {
        1 => rt::StoreAction::Store,
        2 => rt::StoreAction::MultisampleResolve,
        3 => rt::StoreAction::StoreAndMultisampleResolve,
        4 => rt::StoreAction::Unknown,
        _ => rt::StoreAction::DontCare,
    }
}

fn map_rt_error(ledger: &AccuracyLedger, site: &str, err: rt::Error) -> GpuError {
    let gpu_err = match &err {
        rt::Error::Refused(_) => GpuError::new(
            RefusalCode::MetalRefusal,
            12,
            format!("{site}: {err}"),
        ),
        rt::Error::Unsupported(_) | rt::Error::StageUnfilled(_) => GpuError::new(
            RefusalCode::UnsupportedFeature,
            1,
            format!("{site}: {err}"),
        ),
        rt::Error::Translation { .. } => GpuError::new(
            RefusalCode::ShaderTranslation,
            1,
            format!("{site}: {err}"),
        ),
        rt::Error::Quiesced => GpuError::new(
            RefusalCode::DeviceLost,
            1,
            format!("{site}: {err}"),
        ),
        rt::Error::Vulkan { .. }
        | rt::Error::Limit(_)
        | rt::Error::Loader(_)
        | rt::Error::BelowFloor(_)
        | rt::Error::NoDirectDriverLoading
        | rt::Error::NoSuchBlock(_)
        | rt::Error::Thread(_)
        | rt::Error::Fd(_)
        | _ => GpuError::new(
            RefusalCode::VulkanDriver,
            1,
            format!("{site}: {err}"),
        ),
    };
    ledger.record(&gpu_err);
    gpu_err
}

/// Shared helper to satisfy unused `Mutex` import if needed in downstream tests.
#[allow(dead_code)]
fn _touch_mutex() -> Mutex<()> {
    Mutex::new(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn llvm_as_assemble(ir: &str) -> Vec<u8> {
        let mut child = Command::new("llvm-as")
            .args(["-", "-o", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("llvm-as must be installed");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(ir.as_bytes())
            .expect("write IR");
        let output = child.wait_with_output().expect("wait llvm-as");
        assert!(output.status.success(), "llvm-as failed");
        output.stdout
    }

    #[test]
    #[ignore = "requires host Vulkan GPU"]
    fn real_vulkan_compute_and_render_smoke() {
        let plat = RtPlatform::new(None).expect("open RtPlatform on host GPU");
        let caps = plat.caps();
        eprintln!(
            "Opened GPU: {} (unified={}, max_tg_mem={})",
            caps.device_name, caps.has_unified_memory, caps.max_threadgroup_memory_length
        );

        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../vendor/metal2vulkan/crates/metal2vulkan-validation/fixtures/public/examples/runtime_examples.ll");
        let ir = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));
        let bitcode = llvm_as_assemble(&ir);
        let lib = plat.new_library(&bitcode).expect("parse AIR library");

        // 1. Compute test: `scale_and_offset(src, dst, scale, id) -> dst[id] = src[id] * scale + id`
        let pso = plat
            .new_compute_pipeline(&lib, "scale_and_offset", None, false)
            .expect("compile compute pipeline scale_and_offset");
        let block = plat.alloc_block(MemoryClass::Shared, 4096).expect("block");
        let buf_src = plat.new_buffer(&block, 0, 256).expect("buf_src");
        let buf_dst = plat.new_buffer(&block, 256, 256).expect("buf_dst");
        let buf_scale = plat.new_buffer(&block, 512, 256).expect("buf_scale");
        let buf_pos = plat.new_buffer(&block, 768, 256).expect("buf_pos");
        let buf_col = plat.new_buffer(&block, 1024, 256).expect("buf_col");

        let src_bytes: Vec<u8> = (0..64)
            .flat_map(|i| ((i as f32) * 2.0 + 1.0).to_le_bytes())
            .collect();
        let scale = 3.0f32;
        plat.write_buffer(&buf_src, 0, &src_bytes).expect("write src");
        plat.write_buffer(&buf_scale, 0, &scale.to_le_bytes())
            .expect("write scale");

        let queue = plat.new_command_queue(4).expect("queue");
        let mut cb = plat.new_command_buffer(&queue).expect("cb");
        {
            let mut enc = plat.begin_compute_encoder(&mut cb).expect("compute enc");
            enc.set_pipeline(&pso).expect("set pso");
            enc.set_buffer(Some(&buf_src), 0, 0).expect("set src");
            enc.set_buffer(Some(&buf_dst), 0, 1).expect("set dst");
            enc.set_buffer(Some(&buf_scale), 0, 2).expect("set scale");
            enc.dispatch_threads([64, 1, 1], [16, 1, 1])
                .expect("dispatch");
            enc.end_encoding();
        }
        let res = plat.commit_and_wait(cb).expect("commit");
        assert_eq!(res.status, 0);

        let mut out_bytes = vec![0u8; 256];
        plat.read_buffer(&buf_dst, 0, &mut out_bytes)
            .expect("read dst");
        for i in 0..64usize {
            let got = f32::from_le_bytes(out_bytes[i * 4..i * 4 + 4].try_into().unwrap());
            let expected = ((i as f32) * 2.0 + 1.0) * 3.0 + (i as f32);
            assert!(
                (got - expected).abs() < 1e-4,
                "element {i}: got {got}, expected {expected}"
            );
        }

        // 2. Render test: colored triangle into 8x8 BGRA8Unorm texture + readback
        let mut rdesc = RenderPipelineDesc {
            depth_pixel_format: 0,
            stencil_pixel_format: 0,
            sample_count: 1,
            rasterization_enabled: true,
            color_attachments: [Default::default(); 8],
        };
        rdesc.color_attachments[0].pixel_format = 80; // BGRA8Unorm
        let rpso = plat
            .new_render_pipeline(&lib, "colored_vertex", Some(&lib), Some("colored_fragment"), &rdesc)
            .expect("compile render pipeline");

        let positions: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        let colors: [f32; 12] = [
            1.0, 0.25, 0.5, 1.0, 0.25, 1.0, 0.5, 1.0, 0.25, 0.5, 1.0, 1.0,
        ];
        let pos_bytes: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        let col_bytes: Vec<u8> = colors.iter().flat_map(|v| v.to_le_bytes()).collect();
        plat.write_buffer(&buf_pos, 0, &pos_bytes).expect("write pos");
        plat.write_buffer(&buf_col, 0, &col_bytes).expect("write col");

        let tex = plat
            .new_texture(&TextureDesc {
                texture_type: 2, // Type2D
                pixel_format: 80, // BGRA8Unorm
                width: 8,
                height: 8,
                depth: 1,
                mipmap_level_count: 1,
                sample_count: 1,
                array_length: 1,
                usage: 0x05, // ShaderRead | RenderTarget
                storage_mode: 0, // Shared
            })
            .expect("create render target texture");

        let mut cb2 = plat.new_command_buffer(&queue).expect("cb2");
        {
            let empty_att = || crate::core::platform::RenderPassColorAttachment {
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
            atts[0] = crate::core::platform::RenderPassColorAttachment {
                texture: Some(tex.clone()),
                clear_color: [0.0, 0.0, 0.0, 1.0],
                level: 0,
                slice: 0,
                load_action: 2, // Clear
                store_action: 1, // Store
            };
            let pass = RenderPassDesc {
                render_target_width: 8,
                render_target_height: 8,
                default_raster_sample_count: 1,
                color_attachments: atts,
                depth_texture: None,
                clear_depth: 1.0,
                depth_load_action: 0,
                depth_store_action: 0,
            };
            let mut renc = plat.begin_render_encoder(&mut cb2, &pass).expect("render enc");
            renc.set_pipeline(&rpso).expect("set rpso");
            renc.set_vertex_buffer(Some(&buf_pos), 0, 0)
                .expect("set vpos");
            renc.set_vertex_buffer(Some(&buf_col), 0, 1)
                .expect("set vcol");
            renc.draw_primitives(3, 0, 3, 1, 0).expect("draw triangle");
            renc.end_encoding();
        }
        let res2 = plat.commit_and_wait(cb2).expect("commit render");
        assert_eq!(res2.status, 0);

        let mut pixels = vec![0u8; 8 * 8 * 4];
        plat.texture_get_bytes(&tex, [0, 0, 0], [8, 8, 1], 0, 0, &mut pixels, 8 * 4, 8 * 8 * 4)
            .expect("readback rendered texture");
        assert!(
            pixels.iter().any(|&b| b != 0 && b != 255),
            "expected triangle colors in rendered pixels"
        );
    }
}
