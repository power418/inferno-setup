//! Deterministic in-memory `FakePlatform` for unit tests without a Vulkan GPU.

use crate::core::caps::DeviceCaps;
use crate::core::platform::{
    BlitEncoderOps, CommandExecutionResult, ComputeEncoderOps, MemoryClass, Platform,
    RenderEncoderOps, RenderPassDesc, RenderPipelineDesc, ScissorRect, TextureDesc, Viewport,
};
use crate::core::refusal::{AccuracyLedger, GpuError};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// In-memory fake GPU platform.
pub struct FakePlatform {
    caps: DeviceCaps,
    ledger: AccuracyLedger,
    next_id: AtomicU64,
}

impl FakePlatform {
    /// Creates a new `FakePlatform`.
    pub fn new() -> Self {
        Self {
            caps: DeviceCaps {
                device_name: String::from("Inferno FakePlatform (Testkit)"),
                ..DeviceCaps::default()
            },
            ledger: AccuracyLedger::default(),
            next_id: AtomicU64::new(1),
        }
    }

    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}

/// Fake memory block.
#[derive(Clone)]
pub struct FakeBlock {
    data: Arc<Mutex<Vec<u8>>>,
}

/// Fake buffer inside a block.
#[derive(Clone)]
pub struct FakeBuffer {
    block: FakeBlock,
    offset: usize,
    length: usize,
}

/// Fake texture.
#[derive(Clone)]
pub struct FakeTexture {
    desc: TextureDesc,
    data: Arc<Mutex<Vec<u8>>>,
}

/// Fake library.
#[derive(Clone)]
pub struct FakeLibrary {
    bytes: Vec<u8>,
}

/// Fake compute pipeline.
#[derive(Clone)]
pub struct FakeComputePipeline {
    #[allow(dead_code)]
    name: String,
}

/// Fake render pipeline.
#[derive(Clone)]
pub struct FakeRenderPipeline {
    #[allow(dead_code)]
    desc: RenderPipelineDesc,
}

/// Fake queue.
#[derive(Clone)]
pub struct FakeQueue {
    #[allow(dead_code)]
    id: u64,
}

/// Fake command buffer.
pub struct FakeCommandBuffer {
    #[allow(dead_code)]
    id: u64,
}

/// Fake event.
#[derive(Clone)]
pub struct FakeEvent {
    #[allow(dead_code)]
    id: u64,
}

/// Fake fence.
#[derive(Clone)]
pub struct FakeFence {
    #[allow(dead_code)]
    id: u64,
}

struct FakeComputeEnc;
impl ComputeEncoderOps<FakePlatform> for FakeComputeEnc {
    fn set_pipeline(&mut self, _pipeline: &FakeComputePipeline) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_buffer(
        &mut self,
        _buffer: Option<&FakeBuffer>,
        _offset: u64,
        _index: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_texture(
        &mut self,
        _texture: Option<&FakeTexture>,
        _index: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_bytes(&mut self, _bytes: &[u8], _index: u32) -> Result<(), GpuError> {
        Ok(())
    }
    fn dispatch_threads(&mut self, _grid: [u32; 3], _tg: [u32; 3]) -> Result<(), GpuError> {
        Ok(())
    }
    fn dispatch_threadgroups(&mut self, _groups: [u32; 3], _tg: [u32; 3]) -> Result<(), GpuError> {
        Ok(())
    }
    fn update_fence(&mut self, _fence: &FakeFence) -> Result<(), GpuError> {
        Ok(())
    }
    fn wait_for_fence(&mut self, _fence: &FakeFence) -> Result<(), GpuError> {
        Ok(())
    }
    fn end_encoding(self: Box<Self>) {}
}

struct FakeRenderEnc;
impl RenderEncoderOps<FakePlatform> for FakeRenderEnc {
    fn set_pipeline(&mut self, _pipeline: &FakeRenderPipeline) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_viewport(&mut self, _viewport: Viewport) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_scissor(&mut self, _scissor: ScissorRect) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_vertex_buffer(
        &mut self,
        _buffer: Option<&FakeBuffer>,
        _offset: u64,
        _index: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_fragment_buffer(
        &mut self,
        _buffer: Option<&FakeBuffer>,
        _offset: u64,
        _index: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_fragment_texture(
        &mut self,
        _texture: Option<&FakeTexture>,
        _index: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_vertex_bytes(&mut self, _bytes: &[u8], _index: u32) -> Result<(), GpuError> {
        Ok(())
    }
    fn set_fragment_bytes(&mut self, _bytes: &[u8], _index: u32) -> Result<(), GpuError> {
        Ok(())
    }
    fn draw_primitives(
        &mut self,
        _primitive_type: u32,
        _vertex_start: u32,
        _vertex_count: u32,
        _instance_count: u32,
        _base_instance: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn draw_indexed_primitives(
        &mut self,
        _primitive_type: u32,
        _index_count: u32,
        _index_type: u32,
        _index_buffer: &FakeBuffer,
        _index_buffer_offset: u64,
        _instance_count: u32,
        _base_vertex: i32,
        _base_instance: u32,
    ) -> Result<(), GpuError> {
        Ok(())
    }
    fn end_encoding(self: Box<Self>) {}
}

struct FakeBlitEnc;
impl BlitEncoderOps<FakePlatform> for FakeBlitEnc {
    fn copy_buffer(
        &mut self,
        src: &FakeBuffer,
        src_offset: u64,
        dst: &FakeBuffer,
        dst_offset: u64,
        size: u64,
    ) -> Result<(), GpuError> {
        let len = size as usize;
        let s0 = src.offset + src_offset as usize;
        let d0 = dst.offset + dst_offset as usize;
        let bytes = src.block.data.lock()[s0..s0 + len].to_vec();
        dst.block.data.lock()[d0..d0 + len].copy_from_slice(&bytes);
        Ok(())
    }

    fn copy_texture(
        &mut self,
        _src: &FakeTexture,
        _src_slice: u32,
        _src_level: u32,
        _src_origin: [u32; 3],
        _extent: [u32; 3],
        _dst: &FakeTexture,
        _dst_slice: u32,
        _dst_level: u32,
        _dst_origin: [u32; 3],
    ) -> Result<(), GpuError> {
        Ok(())
    }

    fn fill_buffer(
        &mut self,
        buffer: &FakeBuffer,
        offset: u64,
        length: u64,
        value: u8,
    ) -> Result<(), GpuError> {
        let start = buffer.offset + offset as usize;
        let end = start + length as usize;
        let mut guard = buffer.block.data.lock();
        if end > guard.len() {
            return Err(GpuError::invalid("fill_buffer out of bounds"));
        }
        guard[start..end].fill(value);
        Ok(())
    }

    fn end_encoding(self: Box<Self>) {}
}

impl Platform for FakePlatform {
    type Block = FakeBlock;
    type Buffer = FakeBuffer;
    type Texture = FakeTexture;
    type Library = FakeLibrary;
    type ComputePipeline = FakeComputePipeline;
    type RenderPipeline = FakeRenderPipeline;
    type Queue = FakeQueue;
    type CommandBuffer = FakeCommandBuffer;
    type Event = FakeEvent;
    type Fence = FakeFence;

    fn caps(&self) -> &DeviceCaps {
        &self.caps
    }

    fn ledger(&self) -> &AccuracyLedger {
        &self.ledger
    }

    fn alloc_block(&self, _class: MemoryClass, size: u64) -> Result<Self::Block, GpuError> {
        Ok(FakeBlock {
            data: Arc::new(Mutex::new(vec![0u8; size as usize])),
        })
    }

    fn new_buffer(
        &self,
        block: &Self::Block,
        offset: u64,
        length: u64,
    ) -> Result<Self::Buffer, GpuError> {
        let total = block.data.lock().len() as u64;
        if offset.checked_add(length).is_none_or(|end| end > total) {
            return Err(GpuError::invalid("buffer exceeds block"));
        }
        Ok(FakeBuffer {
            block: block.clone(),
            offset: offset as usize,
            length: length as usize,
        })
    }

    fn write_buffer(
        &self,
        buffer: &Self::Buffer,
        offset: u64,
        data: &[u8],
    ) -> Result<(), GpuError> {
        let off = offset as usize;
        if off.checked_add(data.len()).is_none_or(|e| e > buffer.length) {
            return Err(GpuError::invalid("write_buffer out of range"));
        }
        let mut guard = buffer.block.data.lock();
        let start = buffer.offset + off;
        guard[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn read_buffer(
        &self,
        buffer: &Self::Buffer,
        offset: u64,
        out: &mut [u8],
    ) -> Result<(), GpuError> {
        let off = offset as usize;
        if off.checked_add(out.len()).is_none_or(|e| e > buffer.length) {
            return Err(GpuError::invalid("read_buffer out of range"));
        }
        let guard = buffer.block.data.lock();
        let start = buffer.offset + off;
        out.copy_from_slice(&guard[start..start + out.len()]);
        Ok(())
    }

    fn new_texture(&self, desc: &TextureDesc) -> Result<Self::Texture, GpuError> {
        let size = (desc.width.max(1) as usize) * (desc.height.max(1) as usize) * 4;
        Ok(FakeTexture {
            desc: desc.clone(),
            data: Arc::new(Mutex::new(vec![0u8; size])),
        })
    }

    fn texture_replace_region(
        &self,
        texture: &Self::Texture,
        origin: [u32; 3],
        extent: [u32; 3],
        _level: u32,
        _slice: u32,
        bytes: &[u8],
        bytes_per_row: u64,
        _bytes_per_image: u64,
    ) -> Result<(), GpuError> {
        let mut guard = texture.data.lock();
        let dst_stride = texture.desc.width.max(1) as usize * 4;
        for row in 0..extent[1] as usize {
            let src_off = row * bytes_per_row as usize;
            let dst_off =
                ((origin[1] as usize + row) * dst_stride) + (origin[0] as usize * 4);
            let copy_len = (extent[0] as usize * 4)
                .min(bytes.len().saturating_sub(src_off))
                .min(guard.len().saturating_sub(dst_off));
            if copy_len > 0 {
                guard[dst_off..dst_off + copy_len]
                    .copy_from_slice(&bytes[src_off..src_off + copy_len]);
            }
        }
        Ok(())
    }

    fn texture_get_bytes(
        &self,
        texture: &Self::Texture,
        origin: [u32; 3],
        extent: [u32; 3],
        _level: u32,
        _slice: u32,
        out: &mut [u8],
        bytes_per_row: u64,
        _bytes_per_image: u64,
    ) -> Result<(), GpuError> {
        let guard = texture.data.lock();
        let src_stride = texture.desc.width.max(1) as usize * 4;
        for row in 0..extent[1] as usize {
            let src_off =
                ((origin[1] as usize + row) * src_stride) + (origin[0] as usize * 4);
            let dst_off = row * bytes_per_row as usize;
            let copy_len = (extent[0] as usize * 4)
                .min(guard.len().saturating_sub(src_off))
                .min(out.len().saturating_sub(dst_off));
            if copy_len > 0 {
                out[dst_off..dst_off + copy_len]
                    .copy_from_slice(&guard[src_off..src_off + copy_len]);
            }
        }
        Ok(())
    }

    fn new_library(&self, bytes: &[u8]) -> Result<Self::Library, GpuError> {
        if bytes.is_empty() {
            return Err(GpuError::invalid("empty library"));
        }
        Ok(FakeLibrary {
            bytes: bytes.to_vec(),
        })
    }

    fn new_compute_pipeline(
        &self,
        library: &Self::Library,
        function_name: &str,
        _max_total_threads: Option<u32>,
        _threadgroup_size_multiple: bool,
    ) -> Result<Self::ComputePipeline, GpuError> {
        if library.bytes.is_empty() || function_name.is_empty() {
            return Err(GpuError::invalid("invalid compute pipeline"));
        }
        Ok(FakeComputePipeline {
            name: function_name.to_owned(),
        })
    }

    fn new_render_pipeline(
        &self,
        _vertex_lib: &Self::Library,
        vertex_name: &str,
        _fragment_lib: Option<&Self::Library>,
        _fragment_name: Option<&str>,
        desc: &RenderPipelineDesc,
    ) -> Result<Self::RenderPipeline, GpuError> {
        if vertex_name.is_empty() {
            return Err(GpuError::invalid("empty vertex name"));
        }
        Ok(FakeRenderPipeline { desc: desc.clone() })
    }

    fn new_command_queue(&self, _max_command_buffers: u32) -> Result<Self::Queue, GpuError> {
        Ok(FakeQueue {
            id: self.alloc_id(),
        })
    }

    fn new_command_buffer(&self, _queue: &Self::Queue) -> Result<Self::CommandBuffer, GpuError> {
        Ok(FakeCommandBuffer {
            id: self.alloc_id(),
        })
    }

    fn begin_compute_encoder<'a>(
        &'a self,
        _cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn ComputeEncoderOps<Self> + 'a>, GpuError> {
        Ok(Box::new(FakeComputeEnc))
    }

    fn begin_render_encoder<'a>(
        &'a self,
        _cmd_buf: &'a mut Self::CommandBuffer,
        _pass: &RenderPassDesc<Self::Texture>,
    ) -> Result<Box<dyn RenderEncoderOps<Self> + 'a>, GpuError> {
        Ok(Box::new(FakeRenderEnc))
    }

    fn begin_blit_encoder<'a>(
        &'a self,
        _cmd_buf: &'a mut Self::CommandBuffer,
    ) -> Result<Box<dyn BlitEncoderOps<Self> + 'a>, GpuError> {
        Ok(Box::new(FakeBlitEnc))
    }

    fn encode_signal_event(
        &self,
        _cmd_buf: &mut Self::CommandBuffer,
        _event: &Self::Event,
        _value: u64,
    ) -> Result<(), GpuError> {
        Ok(())
    }

    fn encode_wait_event(
        &self,
        _cmd_buf: &mut Self::CommandBuffer,
        _event: &Self::Event,
        _value: u64,
    ) -> Result<(), GpuError> {
        Ok(())
    }

    fn commit_and_wait(
        &self,
        _cmd_buf: Self::CommandBuffer,
    ) -> Result<CommandExecutionResult, GpuError> {
        Ok(CommandExecutionResult {
            status: 0,
            refusal_code: 0,
            gpu_start_ns: 1,
            gpu_end_ns: 2,
        })
    }

    fn new_event(&self) -> Result<Self::Event, GpuError> {
        Ok(FakeEvent {
            id: self.alloc_id(),
        })
    }

    fn new_fence(&self) -> Result<Self::Fence, GpuError> {
        Ok(FakeFence {
            id: self.alloc_id(),
        })
    }

    fn quiesce(&self) {}
}
