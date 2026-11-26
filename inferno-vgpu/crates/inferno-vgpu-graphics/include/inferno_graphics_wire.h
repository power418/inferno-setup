#ifndef INFERNO_GRAPHICS_WIRE_H
#define INFERNO_GRAPHICS_WIRE_H

#pragma once

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IVG_MAGIC               0x49564750u /* "IVGP" */
#define IVG_VERSION_MAJOR       1u
#define IVG_VERSION_MINOR       0u
#define IVG_VERSION             ((IVG_VERSION_MAJOR << 16) | IVG_VERSION_MINOR)

#define IVG_FEAT_UNIFIED_MEMORY   (1ULL << 0)
#define IVG_FEAT_SHARED_POOL      (1ULL << 1)
#define IVG_FEAT_SURFACE_PAGES    (1ULL << 2)
#define IVG_FEAT_DOORBELL_PAGES   (1ULL << 3)
#define IVG_FEAT_DISPLAY_READBACK (1ULL << 4)

/* Region 0 MMIO Register Offsets */
#define IVG_REG_MAGIC             0x000u
#define IVG_REG_VERSION           0x004u
#define IVG_REG_FEATURES_LO       0x008u
#define IVG_REG_FEATURES_HI       0x00Cu
#define IVG_REG_STATUS            0x010u
#define IVG_REG_CONTROL_SUB_OFF_LO 0x018u
#define IVG_REG_CONTROL_SUB_OFF_HI 0x01Cu
#define IVG_REG_CONTROL_DOORBELL  0x020u
#define IVG_REG_QUEUE_DOORBELL    0x024u
#define IVG_REG_IRQ_STATUS        0x028u
#define IVG_REG_IRQ_ACK           0x02Cu
#define IVG_REG_REGION_SIZE_LO    0x030u
#define IVG_REG_REGION_SIZE_HI    0x034u
#define IVG_REG_POOL_BASE_LO      0x038u
#define IVG_REG_POOL_BASE_HI      0x03Cu
#define IVG_REG_POOL_SIZE_LO      0x040u
#define IVG_REG_POOL_SIZE_HI      0x044u
#define IVG_REG_DISPLAY_WIDTH     0x048u
#define IVG_REG_DISPLAY_HEIGHT    0x04Cu
#define IVG_REG_ERROR_CODE        0x050u
#define IVG_REG_CONTROL_CMP_OFF_LO 0x058u
#define IVG_REG_CONTROL_CMP_OFF_HI 0x05Cu

#define IVG_STATUS_RESET          0u
#define IVG_STATUS_DRIVER_READY   (1u << 0)
#define IVG_STATUS_HOST_READY     (1u << 1)
#define IVG_STATUS_DEVICE_LOST    (1u << 31)

#define IVG_IRQ_CONTROL           (1u << 0)
#define IVG_IRQ_QUEUE             (1u << 1)
#define IVG_IRQ_ERROR             (1u << 31)

#define IVG_RING_FLAG_HOST_IDLE   (1u << 0)
#define IVG_RING_FLAG_ARM_IRQ     (1u << 1)

/* Control ring opcodes */
#define IVG_OP_PING                    0x0001u
#define IVG_OP_GET_DEVICE_CAPS         0x0002u
#define IVG_OP_CONTEXT_CREATE          0x0003u
#define IVG_OP_CONTEXT_DESTROY         0x0004u
#define IVG_OP_QUEUE_CREATE            0x0005u
#define IVG_OP_QUEUE_DESTROY           0x0006u
#define IVG_OP_BLOCK_ALLOC             0x0007u
#define IVG_OP_BLOCK_FREE              0x0008u
#define IVG_OP_BUFFER_CREATE           0x0009u
#define IVG_OP_BUFFER_DESTROY          0x000Au
#define IVG_OP_TEXTURE_CREATE          0x000Bu
#define IVG_OP_TEXTURE_DESTROY         0x000Cu
#define IVG_OP_TEXTURE_UPLOAD          0x000Du
#define IVG_OP_TEXTURE_READBACK        0x000Eu
#define IVG_OP_LIBRARY_CREATE          0x000Fu
#define IVG_OP_LIBRARY_DESTROY         0x0010u
#define IVG_OP_PIPELINE_CREATE_COMPUTE 0x0011u
#define IVG_OP_PIPELINE_CREATE_RENDER  0x0012u
#define IVG_OP_PIPELINE_DESTROY        0x0013u
#define IVG_OP_SURFACE_IMPORT          0x0014u
#define IVG_OP_SURFACE_RELEASE         0x0015u
#define IVG_OP_DISPLAY_SET_MODE        0x0016u
#define IVG_OP_DISPLAY_PRESENT         0x0017u
#define IVG_OP_BUFFER_UPLOAD           0x0018u
#define IVG_OP_BUFFER_READBACK         0x0019u
#define IVG_OP_EVENT_CREATE            0x001Au
#define IVG_OP_EVENT_DESTROY           0x001Bu
#define IVG_OP_FENCE_CREATE            0x001Cu
#define IVG_OP_FENCE_DESTROY           0x001Du

/* Queue submission ring opcodes */
#define IVG_QOP_CMD_BUFFER_BEGIN       0x0100u
#define IVG_QOP_CMD_BUFFER_COMMIT      0x0101u
#define IVG_QOP_COMPUTE_BEGIN          0x0200u
#define IVG_QOP_COMPUTE_SET_PIPELINE   0x0201u
#define IVG_QOP_COMPUTE_SET_BUFFER     0x0202u
#define IVG_QOP_COMPUTE_SET_TEXTURE    0x0203u
#define IVG_QOP_COMPUTE_SET_BYTES      0x0204u
#define IVG_QOP_COMPUTE_DISPATCH_THREADS      0x0205u
#define IVG_QOP_COMPUTE_DISPATCH_THREADGROUPS 0x0206u
#define IVG_QOP_COMPUTE_END            0x0207u
#define IVG_QOP_RENDER_BEGIN_PASS      0x0300u
#define IVG_QOP_RENDER_SET_PIPELINE    0x0301u
#define IVG_QOP_RENDER_SET_VIEWPORT    0x0302u
#define IVG_QOP_RENDER_SET_SCISSOR     0x0303u
#define IVG_QOP_RENDER_SET_VERTEX_BUFFER   0x0304u
#define IVG_QOP_RENDER_SET_FRAGMENT_BUFFER 0x0305u
#define IVG_QOP_RENDER_SET_FRAGMENT_TEXTURE 0x0306u
#define IVG_QOP_RENDER_SET_VERTEX_BYTES    0x0307u
#define IVG_QOP_RENDER_SET_FRAGMENT_BYTES  0x0308u
#define IVG_QOP_RENDER_DRAW_PRIMITIVES 0x0309u
#define IVG_QOP_RENDER_DRAW_INDEXED    0x030Au
#define IVG_QOP_RENDER_END_PASS        0x030Bu
#define IVG_QOP_BLIT_BEGIN             0x0400u
#define IVG_QOP_BLIT_COPY_BUFFER       0x0401u
#define IVG_QOP_BLIT_COPY_TEXTURE      0x0402u
#define IVG_QOP_BLIT_FILL_BUFFER       0x0403u
#define IVG_QOP_BLIT_END               0x0404u
#define IVG_QOP_EVENT_SIGNAL           0x0500u
#define IVG_QOP_EVENT_WAIT             0x0501u
#define IVG_QOP_FENCE_UPDATE           0x0502u
#define IVG_QOP_FENCE_WAIT             0x0503u

#ifdef _MSC_VER
#define IVG_ALIGN(N) __declspec(align(N))
#else
#define IVG_ALIGN(N) __attribute__((aligned(N)))
#endif

typedef struct IVG_ALIGN(64) IvgRingHeader {
    uint32_t producer;
    uint32_t consumer;
    uint32_t capacity_mask;
    uint32_t flags;
    uint64_t data_offset;
    uint64_t reserved[5];
} IvgRingHeader;

typedef struct IvgRecordHeader {
    uint16_t opcode;
    uint16_t flags;
    uint32_t size;
    uint32_t context_id;
    uint32_t reserved;
    uint64_t seqno;
} IvgRecordHeader;

typedef struct IvgCompletionRecord {
    uint64_t seqno;
    uint32_t status;
    uint32_t refusal_code;
    uint64_t gpu_start_ns;
    uint64_t gpu_end_ns;
    uint64_t result_u64;
} IvgCompletionRecord;

typedef struct IvgDeviceCaps {
    uint32_t vendor_id;
    uint32_t device_id;
    uint32_t has_unified_memory;
    uint32_t max_threadgroup_memory_length;
    uint64_t max_buffer_length;
    uint32_t read_write_texture_tier;
    uint32_t argument_buffers_tier;
    uint32_t apple_gpu_family;
    uint32_t supports_function_pointers;
    uint8_t  device_name[128];
} IvgDeviceCaps;

typedef struct IvgPingPayload {
    uint64_t cookie;
} IvgPingPayload;

typedef struct IvgGetCapsPayload {
    uint64_t caps_region_offset;
} IvgGetCapsPayload;

typedef struct IvgContextCreatePayload {
    uint32_t pid;
    uint32_t reserved;
} IvgContextCreatePayload;

typedef struct IvgQueueCreatePayload {
    uint64_t queue_id;
    uint64_t sub_ring_offset;
    uint64_t cmp_ring_offset;
    uint32_t max_command_buffers;
    uint32_t reserved;
} IvgQueueCreatePayload;

typedef struct IvgHandlePayload {
    uint64_t handle;
} IvgHandlePayload;

typedef struct IvgBlockAllocPayload {
    uint64_t block_id;
    uint64_t size;
    uint32_t memory_class;
    uint32_t reserved;
} IvgBlockAllocPayload;

typedef struct IvgBufferCreatePayload {
    uint64_t buffer_id;
    uint64_t block_id;
    uint64_t block_offset;
    uint64_t length;
    uint64_t gpu_address;
} IvgBufferCreatePayload;

typedef struct IvgBufferTransferPayload {
    uint64_t buffer_id;
    uint64_t buffer_offset;
    uint64_t region_offset;
    uint64_t length;
} IvgBufferTransferPayload;

typedef struct IvgTextureDescWire {
    uint32_t texture_type;
    uint32_t pixel_format;
    uint32_t width;
    uint32_t height;
    uint32_t depth;
    uint32_t mipmap_level_count;
    uint32_t sample_count;
    uint32_t array_length;
    uint32_t usage;
    uint32_t storage_mode;
} IvgTextureDescWire;

typedef struct IvgTextureCreatePayload {
    uint64_t texture_id;
    IvgTextureDescWire desc;
    uint64_t gpu_resource_id;
} IvgTextureCreatePayload;

typedef struct IvgTextureTransferPayload {
    uint64_t texture_id;
    uint64_t region_offset;
    uint32_t origin[3];
    uint32_t extent[3];
    uint32_t level;
    uint32_t slice;
    uint64_t bytes_per_row;
    uint64_t bytes_per_image;
} IvgTextureTransferPayload;

typedef struct IvgLibraryCreatePayload {
    uint64_t library_id;
    uint8_t  sha256[32];
    uint64_t region_offset;
    uint64_t byte_len;
} IvgLibraryCreatePayload;

typedef struct IvgComputePipelineCreatePayload {
    uint64_t pipeline_id;
    uint64_t library_id;
    uint32_t function_name_len;
    uint32_t max_total_threads_per_threadgroup;
    uint32_t threadgroup_size_multiple;
    uint32_t reserved;
} IvgComputePipelineCreatePayload;

typedef struct IvgRenderColorAttachmentWire {
    uint32_t pixel_format;
    uint8_t  blending_enabled;
    uint8_t  src_rgb_factor;
    uint8_t  dst_rgb_factor;
    uint8_t  rgb_blend_op;
    uint8_t  src_alpha_factor;
    uint8_t  dst_alpha_factor;
    uint8_t  alpha_blend_op;
    uint8_t  write_mask;
} IvgRenderColorAttachmentWire;

typedef struct IvgRenderPipelineCreatePayload {
    uint64_t pipeline_id;
    uint64_t vertex_library_id;
    uint64_t fragment_library_id;
    uint32_t vertex_name_len;
    uint32_t fragment_name_len;
    uint32_t depth_pixel_format;
    uint32_t stencil_pixel_format;
    uint32_t sample_count;
    uint32_t rasterization_enabled;
    IvgRenderColorAttachmentWire color_attachments[8];
} IvgRenderPipelineCreatePayload;

typedef struct IvgSurfaceImportPayload {
    uint64_t texture_id;
    uint32_t surface_id;
    uint32_t plane;
    IvgTextureDescWire desc;
    uint64_t region_offset;
    uint64_t bytes_per_row;
} IvgSurfaceImportPayload;

typedef struct IvgDisplaySetModePayload {
    uint32_t width;
    uint32_t height;
    uint32_t refresh_hz;
    uint32_t reserved;
} IvgDisplaySetModePayload;

typedef struct IvgDisplayPresentPayload {
    uint64_t texture_id;
    uint32_t level;
    uint32_t slice;
} IvgDisplayPresentPayload;

typedef struct IvgCmdBufferBeginPayload {
    uint64_t cmd_buf_id;
} IvgCmdBufferBeginPayload;

typedef struct IvgCmdBufferCommitPayload {
    uint64_t cmd_buf_id;
    uint32_t want_completion;
    uint32_t reserved;
} IvgCmdBufferCommitPayload;

typedef struct IvgComputeSetPipelinePayload {
    uint64_t pipeline_id;
} IvgComputeSetPipelinePayload;

typedef struct IvgComputeSetBufferPayload {
    uint64_t buffer_id;
    uint64_t offset;
    uint32_t index;
    uint32_t reserved;
} IvgComputeSetBufferPayload;

typedef struct IvgComputeSetTexturePayload {
    uint64_t texture_id;
    uint32_t index;
    uint32_t reserved;
} IvgComputeSetTexturePayload;

typedef struct IvgSetBytesPayload {
    uint32_t index;
    uint32_t length;
} IvgSetBytesPayload;

typedef struct IvgComputeDispatchPayload {
    uint32_t grid[3];
    uint32_t threads_per_threadgroup[3];
} IvgComputeDispatchPayload;

typedef struct IvgRenderPassColorAttachmentWire {
    uint64_t texture_id;
    double   clear_color[4];
    uint32_t level;
    uint32_t slice;
    uint32_t load_action;
    uint32_t store_action;
} IvgRenderPassColorAttachmentWire;

typedef struct IvgRenderBeginPassPayload {
    uint32_t render_target_width;
    uint32_t render_target_height;
    uint32_t default_raster_sample_count;
    uint32_t reserved;
    IvgRenderPassColorAttachmentWire color_attachments[8];
    uint64_t depth_texture_id;
    double   clear_depth;
    uint32_t depth_load_action;
    uint32_t depth_store_action;
} IvgRenderBeginPassPayload;

typedef struct IvgRenderSetPipelinePayload {
    uint64_t pipeline_id;
} IvgRenderSetPipelinePayload;

typedef struct IvgRenderSetViewportPayload {
    double origin_x;
    double origin_y;
    double width;
    double height;
    double znear;
    double zfar;
} IvgRenderSetViewportPayload;

typedef struct IvgRenderSetScissorPayload {
    uint32_t x;
    uint32_t y;
    uint32_t width;
    uint32_t height;
} IvgRenderSetScissorPayload;

typedef struct IvgRenderSetBufferPayload {
    uint64_t buffer_id;
    uint64_t offset;
    uint32_t index;
    uint32_t reserved;
} IvgRenderSetBufferPayload;

typedef struct IvgRenderSetTexturePayload {
    uint64_t texture_id;
    uint32_t index;
    uint32_t reserved;
} IvgRenderSetTexturePayload;

typedef struct IvgRenderDrawPrimitivesPayload {
    uint32_t primitive_type;
    uint32_t vertex_start;
    uint32_t vertex_count;
    uint32_t instance_count;
    uint32_t base_instance;
    uint32_t reserved;
} IvgRenderDrawPrimitivesPayload;

typedef struct IvgRenderDrawIndexedPayload {
    uint32_t primitive_type;
    uint32_t index_count;
    uint32_t index_type;
    uint32_t instance_count;
    uint64_t index_buffer_id;
    uint64_t index_buffer_offset;
    int32_t  base_vertex;
    uint32_t base_instance;
} IvgRenderDrawIndexedPayload;

typedef struct IvgBlitCopyBufferPayload {
    uint64_t src_buffer_id;
    uint64_t src_offset;
    uint64_t dst_buffer_id;
    uint64_t dst_offset;
    uint64_t size;
} IvgBlitCopyBufferPayload;

typedef struct IvgBlitCopyTexturePayload {
    uint64_t src_texture_id;
    uint32_t src_slice;
    uint32_t src_level;
    uint32_t src_origin[3];
    uint32_t extent[3];
    uint64_t dst_texture_id;
    uint32_t dst_slice;
    uint32_t dst_level;
    uint32_t dst_origin[3];
    uint32_t reserved;
} IvgBlitCopyTexturePayload;

typedef struct IvgBlitFillBufferPayload {
    uint64_t buffer_id;
    uint64_t offset;
    uint64_t length;
    uint8_t  value;
    uint8_t  reserved[7];
} IvgBlitFillBufferPayload;

typedef struct IvgSyncPayload {
    uint64_t handle;
    uint64_t value;
} IvgSyncPayload;

#ifdef __cplusplus
}
#endif

#endif /* INFERNO_GRAPHICS_WIRE_H */
