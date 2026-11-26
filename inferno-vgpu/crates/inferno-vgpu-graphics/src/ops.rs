//! Untrusted-input wire record decoders and test encoders.

use crate::wire::*;
use core::mem::size_of;
use zerocopy::{FromBytes, Immutable, IntoBytes};

/// Error returned when decoding a wire record fails validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Record is smaller than `IvgRecordHeader` or payload struct.
    Truncated,
    /// Record `size` header field does not match the input buffer length.
    SizeMismatch,
    /// Inline trailing bytes (string or byte slice) exceed record bounds or are invalid UTF-8.
    BadTrailingData,
    /// Unknown opcode.
    UnknownOpcode(u16),
}

/// Decoded control-ring operation (Ring 0).
#[derive(Clone, Debug, PartialEq)]
pub enum ControlOp {
    /// `OP_PING`.
    Ping(IvgPingPayload),
    /// `OP_GET_DEVICE_CAPS`.
    GetDeviceCaps(IvgGetCapsPayload),
    /// `OP_CONTEXT_CREATE`.
    ContextCreate(IvgContextCreatePayload),
    /// `OP_CONTEXT_DESTROY`.
    ContextDestroy,
    /// `OP_QUEUE_CREATE`.
    QueueCreate(IvgQueueCreatePayload),
    /// `OP_QUEUE_DESTROY`.
    QueueDestroy(IvgHandlePayload),
    /// `OP_BLOCK_ALLOC`.
    BlockAlloc(IvgBlockAllocPayload),
    /// `OP_BLOCK_FREE`.
    BlockFree(IvgHandlePayload),
    /// `OP_BUFFER_CREATE`.
    BufferCreate(IvgBufferCreatePayload),
    /// `OP_BUFFER_DESTROY`.
    BufferDestroy(IvgHandlePayload),
    /// `OP_BUFFER_UPLOAD`.
    BufferUpload(IvgBufferTransferPayload),
    /// `OP_BUFFER_READBACK`.
    BufferReadback(IvgBufferTransferPayload),
    /// `OP_TEXTURE_CREATE`.
    TextureCreate(IvgTextureCreatePayload),
    /// `OP_TEXTURE_DESTROY`.
    TextureDestroy(IvgHandlePayload),
    /// `OP_TEXTURE_UPLOAD`.
    TextureUpload(IvgTextureTransferPayload),
    /// `OP_TEXTURE_READBACK`.
    TextureReadback(IvgTextureTransferPayload),
    /// `OP_LIBRARY_CREATE`.
    LibraryCreate(IvgLibraryCreatePayload),
    /// `OP_LIBRARY_DESTROY`.
    LibraryDestroy(IvgHandlePayload),
    /// `OP_PIPELINE_CREATE_COMPUTE`.
    PipelineCreateCompute {
        /// Fixed descriptor fields.
        payload: IvgComputePipelineCreatePayload,
        /// Kernel entry point name.
        function_name: String,
    },
    /// `OP_PIPELINE_CREATE_RENDER`.
    PipelineCreateRender {
        /// Fixed descriptor fields.
        payload: IvgRenderPipelineCreatePayload,
        /// Vertex function name.
        vertex_name: String,
        /// Optional fragment function name.
        fragment_name: Option<String>,
    },
    /// `OP_PIPELINE_DESTROY`.
    PipelineDestroy(IvgHandlePayload),
    /// `OP_SURFACE_IMPORT`.
    SurfaceImport(IvgSurfaceImportPayload),
    /// `OP_SURFACE_RELEASE`.
    SurfaceRelease(IvgHandlePayload),
    /// `OP_DISPLAY_SET_MODE`.
    DisplaySetMode(IvgDisplaySetModePayload),
    /// `OP_DISPLAY_PRESENT`.
    DisplayPresent(IvgDisplayPresentPayload),
    /// `OP_EVENT_CREATE`.
    EventCreate(IvgHandlePayload),
    /// `OP_EVENT_DESTROY`.
    EventDestroy(IvgHandlePayload),
    /// `OP_FENCE_CREATE`.
    FenceCreate(IvgHandlePayload),
    /// `OP_FENCE_DESTROY`.
    FenceDestroy(IvgHandlePayload),
}

/// Decoded command-queue ring operation (Ring 1..N).
#[derive(Clone, Debug, PartialEq)]
pub enum QueueOp {
    /// `CMD_BUFFER_BEGIN`.
    CmdBufferBegin(IvgCmdBufferBeginPayload),
    /// `CMD_BUFFER_COMMIT`.
    CmdBufferCommit(IvgCmdBufferCommitPayload),
    /// `COMPUTE_BEGIN`.
    ComputeBegin,
    /// `COMPUTE_SET_PIPELINE`.
    ComputeSetPipeline(IvgComputeSetPipelinePayload),
    /// `COMPUTE_SET_BUFFER`.
    ComputeSetBuffer(IvgComputeSetBufferPayload),
    /// `COMPUTE_SET_TEXTURE`.
    ComputeSetTexture(IvgComputeSetTexturePayload),
    /// `COMPUTE_SET_BYTES`.
    ComputeSetBytes {
        /// Slot index.
        index: u32,
        /// Inline bytes.
        bytes: Vec<u8>,
    },
    /// `COMPUTE_DISPATCH_THREADS`.
    ComputeDispatchThreads(IvgComputeDispatchPayload),
    /// `COMPUTE_DISPATCH_THREADGROUPS`.
    ComputeDispatchThreadgroups(IvgComputeDispatchPayload),
    /// `COMPUTE_END`.
    ComputeEnd,
    /// `RENDER_BEGIN_PASS`.
    RenderBeginPass(Box<IvgRenderBeginPassPayload>),
    /// `RENDER_SET_PIPELINE`.
    RenderSetPipeline(IvgRenderSetPipelinePayload),
    /// `RENDER_SET_VIEWPORT`.
    RenderSetViewport(IvgRenderSetViewportPayload),
    /// `RENDER_SET_SCISSOR`.
    RenderSetScissor(IvgRenderSetScissorPayload),
    /// `RENDER_SET_VERTEX_BUFFER`.
    RenderSetVertexBuffer(IvgRenderSetBufferPayload),
    /// `RENDER_SET_FRAGMENT_BUFFER`.
    RenderSetFragmentBuffer(IvgRenderSetBufferPayload),
    /// `RENDER_SET_FRAGMENT_TEXTURE`.
    RenderSetFragmentTexture(IvgRenderSetTexturePayload),
    /// `RENDER_SET_VERTEX_BYTES`.
    RenderSetVertexBytes {
        /// Slot index.
        index: u32,
        /// Inline bytes.
        bytes: Vec<u8>,
    },
    /// `RENDER_SET_FRAGMENT_BYTES`.
    RenderSetFragmentBytes {
        /// Slot index.
        index: u32,
        /// Inline bytes.
        bytes: Vec<u8>,
    },
    /// `RENDER_DRAW_PRIMITIVES`.
    RenderDrawPrimitives(IvgRenderDrawPrimitivesPayload),
    /// `RENDER_DRAW_INDEXED`.
    RenderDrawIndexed(IvgRenderDrawIndexedPayload),
    /// `RENDER_END_PASS`.
    RenderEndPass,
    /// `BLIT_BEGIN`.
    BlitBegin,
    /// `BLIT_COPY_BUFFER`.
    BlitCopyBuffer(IvgBlitCopyBufferPayload),
    /// `BLIT_COPY_TEXTURE`.
    BlitCopyTexture(IvgBlitCopyTexturePayload),
    /// `BLIT_FILL_BUFFER`.
    BlitFillBuffer(IvgBlitFillBufferPayload),
    /// `BLIT_END`.
    BlitEnd,
    /// `EVENT_SIGNAL`.
    EventSignal(IvgSyncPayload),
    /// `EVENT_WAIT`.
    EventWait(IvgSyncPayload),
    /// `FENCE_UPDATE`.
    FenceUpdate(IvgSyncPayload),
    /// `FENCE_WAIT`.
    FenceWait(IvgSyncPayload),
}

fn split_header(raw: &[u8]) -> Result<(IvgRecordHeader, &[u8]), DecodeError> {
    if raw.len() < size_of::<IvgRecordHeader>() {
        return Err(DecodeError::Truncated);
    }
    let (hdr_bytes, body) = raw.split_at(size_of::<IvgRecordHeader>());
    let hdr = IvgRecordHeader::read_from_bytes(hdr_bytes).map_err(|_| DecodeError::Truncated)?;
    if hdr.size as usize != raw.len() {
        return Err(DecodeError::SizeMismatch);
    }
    Ok((hdr, body))
}

fn read_pod<T: FromBytes + Immutable>(body: &[u8]) -> Result<(T, &[u8]), DecodeError> {
    if body.len() < size_of::<T>() {
        return Err(DecodeError::Truncated);
    }
    let (head, tail) = body.split_at(size_of::<T>());
    let val = T::read_from_bytes(head).map_err(|_| DecodeError::Truncated)?;
    Ok((val, tail))
}

/// Decodes a control-ring record from raw bytes.
pub fn decode_control(raw: &[u8]) -> Result<(IvgRecordHeader, ControlOp), DecodeError> {
    let (hdr, body) = split_header(raw)?;
    let op = match hdr.opcode {
        control_op::PING => ControlOp::Ping(read_pod(body)?.0),
        control_op::GET_DEVICE_CAPS => ControlOp::GetDeviceCaps(read_pod(body)?.0),
        control_op::CONTEXT_CREATE => ControlOp::ContextCreate(read_pod(body)?.0),
        control_op::CONTEXT_DESTROY => ControlOp::ContextDestroy,
        control_op::QUEUE_CREATE => ControlOp::QueueCreate(read_pod(body)?.0),
        control_op::QUEUE_DESTROY => ControlOp::QueueDestroy(read_pod(body)?.0),
        control_op::BLOCK_ALLOC => ControlOp::BlockAlloc(read_pod(body)?.0),
        control_op::BLOCK_FREE => ControlOp::BlockFree(read_pod(body)?.0),
        control_op::BUFFER_CREATE => ControlOp::BufferCreate(read_pod(body)?.0),
        control_op::BUFFER_DESTROY => ControlOp::BufferDestroy(read_pod(body)?.0),
        control_op::BUFFER_UPLOAD => ControlOp::BufferUpload(read_pod(body)?.0),
        control_op::BUFFER_READBACK => ControlOp::BufferReadback(read_pod(body)?.0),
        control_op::TEXTURE_CREATE => ControlOp::TextureCreate(read_pod(body)?.0),
        control_op::TEXTURE_DESTROY => ControlOp::TextureDestroy(read_pod(body)?.0),
        control_op::TEXTURE_UPLOAD => ControlOp::TextureUpload(read_pod(body)?.0),
        control_op::TEXTURE_READBACK => ControlOp::TextureReadback(read_pod(body)?.0),
        control_op::LIBRARY_CREATE => ControlOp::LibraryCreate(read_pod(body)?.0),
        control_op::LIBRARY_DESTROY => ControlOp::LibraryDestroy(read_pod(body)?.0),
        control_op::PIPELINE_CREATE_COMPUTE => {
            let (payload, tail): (IvgComputePipelineCreatePayload, _) = read_pod(body)?;
            let name_len = payload.function_name_len as usize;
            if tail.len() < name_len {
                return Err(DecodeError::BadTrailingData);
            }
            let name = core::str::from_utf8(&tail[..name_len])
                .map_err(|_| DecodeError::BadTrailingData)?
                .to_owned();
            ControlOp::PipelineCreateCompute {
                payload,
                function_name: name,
            }
        }
        control_op::PIPELINE_CREATE_RENDER => {
            let (payload, tail): (IvgRenderPipelineCreatePayload, _) = read_pod(body)?;
            let vlen = payload.vertex_name_len as usize;
            let flen = payload.fragment_name_len as usize;
            let total = vlen.checked_add(flen).ok_or(DecodeError::BadTrailingData)?;
            if tail.len() < total {
                return Err(DecodeError::BadTrailingData);
            }
            let vertex_name = core::str::from_utf8(&tail[..vlen])
                .map_err(|_| DecodeError::BadTrailingData)?
                .to_owned();
            let fragment_name = if flen > 0 {
                Some(
                    core::str::from_utf8(&tail[vlen..vlen + flen])
                        .map_err(|_| DecodeError::BadTrailingData)?
                        .to_owned(),
                )
            } else {
                None
            };
            ControlOp::PipelineCreateRender {
                payload,
                vertex_name,
                fragment_name,
            }
        }
        control_op::PIPELINE_DESTROY => ControlOp::PipelineDestroy(read_pod(body)?.0),
        control_op::SURFACE_IMPORT => ControlOp::SurfaceImport(read_pod(body)?.0),
        control_op::SURFACE_RELEASE => ControlOp::SurfaceRelease(read_pod(body)?.0),
        control_op::DISPLAY_SET_MODE => ControlOp::DisplaySetMode(read_pod(body)?.0),
        control_op::DISPLAY_PRESENT => ControlOp::DisplayPresent(read_pod(body)?.0),
        control_op::EVENT_CREATE => ControlOp::EventCreate(read_pod(body)?.0),
        control_op::EVENT_DESTROY => ControlOp::EventDestroy(read_pod(body)?.0),
        control_op::FENCE_CREATE => ControlOp::FenceCreate(read_pod(body)?.0),
        control_op::FENCE_DESTROY => ControlOp::FenceDestroy(read_pod(body)?.0),
        other => return Err(DecodeError::UnknownOpcode(other)),
    };
    Ok((hdr, op))
}

/// Decodes a queue submission record from raw bytes.
pub fn decode_queue(raw: &[u8]) -> Result<(IvgRecordHeader, QueueOp), DecodeError> {
    let (hdr, body) = split_header(raw)?;
    let op = match hdr.opcode {
        queue_op::CMD_BUFFER_BEGIN => QueueOp::CmdBufferBegin(read_pod(body)?.0),
        queue_op::CMD_BUFFER_COMMIT => QueueOp::CmdBufferCommit(read_pod(body)?.0),
        queue_op::COMPUTE_BEGIN => QueueOp::ComputeBegin,
        queue_op::COMPUTE_SET_PIPELINE => QueueOp::ComputeSetPipeline(read_pod(body)?.0),
        queue_op::COMPUTE_SET_BUFFER => QueueOp::ComputeSetBuffer(read_pod(body)?.0),
        queue_op::COMPUTE_SET_TEXTURE => QueueOp::ComputeSetTexture(read_pod(body)?.0),
        queue_op::COMPUTE_SET_BYTES => {
            let (p, tail): (IvgSetBytesPayload, _) = read_pod(body)?;
            let len = p.length as usize;
            if tail.len() < len {
                return Err(DecodeError::BadTrailingData);
            }
            QueueOp::ComputeSetBytes {
                index: p.index,
                bytes: tail[..len].to_vec(),
            }
        }
        queue_op::COMPUTE_DISPATCH_THREADS => QueueOp::ComputeDispatchThreads(read_pod(body)?.0),
        queue_op::COMPUTE_DISPATCH_THREADGROUPS => {
            QueueOp::ComputeDispatchThreadgroups(read_pod(body)?.0)
        }
        queue_op::COMPUTE_END => QueueOp::ComputeEnd,
        queue_op::RENDER_BEGIN_PASS => QueueOp::RenderBeginPass(Box::new(read_pod(body)?.0)),
        queue_op::RENDER_SET_PIPELINE => QueueOp::RenderSetPipeline(read_pod(body)?.0),
        queue_op::RENDER_SET_VIEWPORT => QueueOp::RenderSetViewport(read_pod(body)?.0),
        queue_op::RENDER_SET_SCISSOR => QueueOp::RenderSetScissor(read_pod(body)?.0),
        queue_op::RENDER_SET_VERTEX_BUFFER => QueueOp::RenderSetVertexBuffer(read_pod(body)?.0),
        queue_op::RENDER_SET_FRAGMENT_BUFFER => QueueOp::RenderSetFragmentBuffer(read_pod(body)?.0),
        queue_op::RENDER_SET_FRAGMENT_TEXTURE => {
            QueueOp::RenderSetFragmentTexture(read_pod(body)?.0)
        }
        queue_op::RENDER_SET_VERTEX_BYTES => {
            let (p, tail): (IvgSetBytesPayload, _) = read_pod(body)?;
            let len = p.length as usize;
            if tail.len() < len {
                return Err(DecodeError::BadTrailingData);
            }
            QueueOp::RenderSetVertexBytes {
                index: p.index,
                bytes: tail[..len].to_vec(),
            }
        }
        queue_op::RENDER_SET_FRAGMENT_BYTES => {
            let (p, tail): (IvgSetBytesPayload, _) = read_pod(body)?;
            let len = p.length as usize;
            if tail.len() < len {
                return Err(DecodeError::BadTrailingData);
            }
            QueueOp::RenderSetFragmentBytes {
                index: p.index,
                bytes: tail[..len].to_vec(),
            }
        }
        queue_op::RENDER_DRAW_PRIMITIVES => QueueOp::RenderDrawPrimitives(read_pod(body)?.0),
        queue_op::RENDER_DRAW_INDEXED => QueueOp::RenderDrawIndexed(read_pod(body)?.0),
        queue_op::RENDER_END_PASS => QueueOp::RenderEndPass,
        queue_op::BLIT_BEGIN => QueueOp::BlitBegin,
        queue_op::BLIT_COPY_BUFFER => QueueOp::BlitCopyBuffer(read_pod(body)?.0),
        queue_op::BLIT_COPY_TEXTURE => QueueOp::BlitCopyTexture(read_pod(body)?.0),
        queue_op::BLIT_FILL_BUFFER => QueueOp::BlitFillBuffer(read_pod(body)?.0),
        queue_op::BLIT_END => QueueOp::BlitEnd,
        queue_op::EVENT_SIGNAL => QueueOp::EventSignal(read_pod(body)?.0),
        queue_op::EVENT_WAIT => QueueOp::EventWait(read_pod(body)?.0),
        queue_op::FENCE_UPDATE => QueueOp::FenceUpdate(read_pod(body)?.0),
        queue_op::FENCE_WAIT => QueueOp::FenceWait(read_pod(body)?.0),
        other => return Err(DecodeError::UnknownOpcode(other)),
    };
    Ok((hdr, op))
}

/// Encodes a fixed-payload record into a byte vector.
pub fn encode_record<T: IntoBytes + Immutable>(
    opcode: u16,
    context_id: u32,
    seqno: u64,
    payload: &T,
) -> Vec<u8> {
    encode_record_with_tail(opcode, context_id, seqno, payload.as_bytes(), &[])
}

/// Encodes a record with arbitrary body and trailing bytes.
pub fn encode_record_with_tail(
    opcode: u16,
    context_id: u32,
    seqno: u64,
    body: &[u8],
    tail: &[u8],
) -> Vec<u8> {
    let total = size_of::<IvgRecordHeader>() + body.len() + tail.len();
    let hdr = IvgRecordHeader {
        opcode,
        flags: 0,
        size: total as u32,
        context_id,
        reserved: 0,
        seqno,
    };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(hdr.as_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_control_and_queue_ops() {
        let ping = encode_record(
            control_op::PING,
            7,
            42,
            &IvgPingPayload { cookie: 0xDEADBEEF },
        );
        let (hdr, op) = decode_control(&ping).expect("decode ping");
        assert_eq!(hdr.context_id, 7);
        assert_eq!(hdr.seqno, 42);
        assert_eq!(op, ControlOp::Ping(IvgPingPayload { cookie: 0xDEADBEEF }));

        let pso = IvgComputePipelineCreatePayload {
            pipeline_id: 10,
            library_id: 20,
            function_name_len: 10,
            max_total_threads_per_threadgroup: 256,
            threadgroup_size_multiple: 1,
            reserved: 0,
        };
        let pso_raw = encode_record_with_tail(
            control_op::PIPELINE_CREATE_COMPUTE,
            7,
            43,
            pso.as_bytes(),
            b"vec_add_k0",
        );
        let (_, op) = decode_control(&pso_raw).expect("decode compute pso");
        assert!(matches!(
            op,
            ControlOp::PipelineCreateCompute { function_name, .. } if function_name == "vec_add_k0"
        ));

        let draw = encode_record(
            queue_op::RENDER_DRAW_PRIMITIVES,
            7,
            44,
            &IvgRenderDrawPrimitivesPayload {
                primitive_type: 3,
                vertex_start: 0,
                vertex_count: 3,
                instance_count: 1,
                base_instance: 0,
                reserved: 0,
            },
        );
        let (_, qop) = decode_queue(&draw).expect("decode draw");
        assert!(matches!(qop, QueueOp::RenderDrawPrimitives(_)));
    }

    #[test]
    fn adversarial_bytes_never_panic() {
        for len in 0..64 {
            let mut junk = vec![0xFFu8; len];
            let _ = decode_control(&junk);
            let _ = decode_queue(&junk);
            if len >= size_of::<IvgRecordHeader>() {
                // Match size header to test body truncation & invalid UTF-8 paths.
                junk[4..8].copy_from_slice(&(len as u32).to_le_bytes());
                for opcode in [
                    control_op::PING,
                    control_op::PIPELINE_CREATE_COMPUTE,
                    control_op::PIPELINE_CREATE_RENDER,
                ] {
                    junk[0..2].copy_from_slice(&opcode.to_le_bytes());
                    let _ = decode_control(&junk);
                }
                for opcode in [
                    queue_op::COMPUTE_SET_BYTES,
                    queue_op::RENDER_BEGIN_PASS,
                    queue_op::RENDER_DRAW_INDEXED,
                ] {
                    junk[0..2].copy_from_slice(&opcode.to_le_bytes());
                    let _ = decode_queue(&junk);
                }
            }
        }
    }
}
