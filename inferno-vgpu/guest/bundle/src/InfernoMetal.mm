#import "InfernoMetal.h"
#import "InfernoWireClient.hpp"
#import <IOSurface/IOSurface.h>
#import <IOKit/IOKitLib.h>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <unistd.h>

using inferno::gpu::WireClient;
using inferno::gpu::WireRing;

static bool spi_log_enabled() {
    static bool enabled = (std::getenv("INFERNO_METAL_LOG_SPI") != nullptr);
    return enabled;
}

#define LOG_SPI(fmt, ...) \
    do { if (spi_log_enabled()) NSLog(@"[InfernoMetal SPI] " fmt, ##__VA_ARGS__); } while (0)

@interface InfernoMTLDevice : NSObject <MTLDevice>
@property(nonatomic, readonly) WireClient* client;
@end

@interface InfernoMTLBuffer : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, readonly) uint64_t blockHandle;
@property(nonatomic, readonly) NSUInteger length;
@property(nonatomic, readonly) MTLResourceOptions resourceOptions;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLTexture : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, readonly) MTLPixelFormat pixelFormat;
@property(nonatomic, readonly) NSUInteger width;
@property(nonatomic, readonly) NSUInteger height;
@property(nonatomic, readonly) MTLTextureType textureType;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLLibrary : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLFunction : NSObject
@property(nonatomic, readonly) uint64_t libraryHandle;
@property(nonatomic, readonly, copy) NSString* name;
@property(nonatomic, readonly) MTLFunctionType functionType;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLComputePipelineState : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLRenderPipelineState : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
@end

@interface InfernoMTLCommandQueue : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, weak, readonly) InfernoMTLDevice* device;
- (WireRing&)subRing;
- (WireRing&)cmpRing;
@end

@interface InfernoMTLCommandBuffer : NSObject
@property(nonatomic, readonly) uint64_t handle;
@property(nonatomic, strong, readonly) InfernoMTLCommandQueue* queue;
- (void)appendRecord:(uint16_t)opcode payload:(const void*)payload len:(size_t)len tail:(const void*)tail tailLen:(size_t)tailLen;
@end

@interface InfernoMTLComputeCommandEncoder : NSObject
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf;
@end

@interface InfernoMTLRenderCommandEncoder : NSObject
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf descriptor:(MTLRenderPassDescriptor*)desc;
@end

@interface InfernoMTLBlitCommandEncoder : NSObject
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf;
@end

/* --- Transport singleton connecting to InfernoVgpuUserClient --- */
namespace {
struct GuestTransport {
    io_connect_t conn = IO_OBJECT_NULL;
    uint8_t* region = nullptr;
    size_t region_size = 0;
    std::unique_ptr<WireClient> client;

    static GuestTransport& instance() {
        static GuestTransport inst;
        static dispatch_once_t once;
        dispatch_once(&once, ^{ inst.connect(); });
        return inst;
    }

    void connect() {
        io_service_t svc = IOServiceGetMatchingService(kIOMasterPortDefault, IOServiceMatching("InfernoVgpuService"));
        if (svc == IO_OBJECT_NULL) {
            LOG_SPI(@"InfernoVgpuService not found in IORegistry");
            return;
        }
        if (IOServiceOpen(svc, mach_task_self(), 0, &conn) != KERN_SUCCESS) {
            IOObjectRelease(svc);
            return;
        }
        IOObjectRelease(svc);

        mach_vm_address_t addr = 0;
        mach_vm_size_t    size = 0;
        if (IOConnectMapMemory(conn, 0, mach_task_self(), &addr, &size, kIOMapAnywhere) != KERN_SUCCESS) {
            return;
        }
        region      = reinterpret_cast<uint8_t*>(addr);
        region_size = static_cast<size_t>(size);

        uint32_t ctx_id = static_cast<uint32_t>(getpid());
        client = std::make_unique<WireClient>(region, region_size, ctx_id, [this](uint64_t qid) {
            uint64_t input = qid;
            IOConnectCallScalarMethod(conn, 0 /* kDoorbellMethod */, &input, 1, nullptr, nullptr);
        });
        client->initialize_control_rings(0x0, 0x1000, 0x10000, 0x11000, 16384);
        client->create_context(ctx_id);
    }
};
} // namespace

@implementation InfernoMTLBuffer {
    std::vector<uint8_t> _shadow;
}
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h block:(uint64_t)blk length:(NSUInteger)len options:(MTLResourceOptions)opt {
    if ((self = [super init])) {
        _device = dev;
        _handle = h;
        _blockHandle = blk;
        _length = len;
        _resourceOptions = opt;
        _shadow.resize(len, 0);
    }
    return self;
}
- (void*)contents { return _shadow.data(); }
- (void)didModifyRange:(NSRange)range {
    if (range.location + range.length > _shadow.size()) return;
    size_t stg = _device.client->alloc_staging(range.length);
    std::memcpy(_device.client->region_base() + stg, _shadow.data() + range.location, range.length);
    IvgBufferTransferPayload p{_handle, range.location, static_cast<uint64_t>(stg), range.length};
    IvgCompletionRecord cmp{};
    _device.client->submit_control_pod(IVG_OP_BUFFER_UPLOAD, &p, sizeof(p), cmp);
}
@end

@implementation InfernoMTLTexture
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h desc:(MTLTextureDescriptor*)desc {
    if ((self = [super init])) {
        _device = dev;
        _handle = h;
        _pixelFormat = desc.pixelFormat;
        _width = desc.width;
        _height = desc.height;
        _textureType = desc.textureType;
    }
    return self;
}
- (void)replaceRegion:(MTLRegion)region mipmapLevel:(NSUInteger)level slice:(NSUInteger)slice withBytes:(const void*)bytes bytesPerRow:(NSUInteger)bpr bytesPerImage:(NSUInteger)bpi {
    size_t total = bpi ? bpi : (bpr * region.size.height);
    size_t stg = _device.client->alloc_staging(total);
    std::memcpy(_device.client->region_base() + stg, bytes, total);
    IvgTextureTransferPayload p{};
    p.texture_id      = _handle;
    p.region_offset   = stg;
    p.origin[0]       = (uint32_t)region.origin.x;
    p.origin[1]       = (uint32_t)region.origin.y;
    p.origin[2]       = (uint32_t)region.origin.z;
    p.extent[0]       = (uint32_t)region.size.width;
    p.extent[1]       = (uint32_t)region.size.height;
    p.extent[2]       = (uint32_t)region.size.depth;
    p.level           = (uint32_t)level;
    p.slice           = (uint32_t)slice;
    p.bytes_per_row   = bpr;
    p.bytes_per_image = bpi;
    IvgCompletionRecord cmp{};
    _device.client->submit_control_pod(IVG_OP_TEXTURE_UPLOAD, &p, sizeof(p), cmp);
}
- (void)getBytes:(void*)pixelBytes bytesPerRow:(NSUInteger)bpr bytesPerImage:(NSUInteger)bpi fromRegion:(MTLRegion)region mipmapLevel:(NSUInteger)level slice:(NSUInteger)slice {
    size_t total = bpi ? bpi : (bpr * region.size.height);
    size_t stg = _device.client->alloc_staging(total);
    IvgTextureTransferPayload p{};
    p.texture_id      = _handle;
    p.region_offset   = stg;
    p.origin[0]       = (uint32_t)region.origin.x;
    p.origin[1]       = (uint32_t)region.origin.y;
    p.origin[2]       = (uint32_t)region.origin.z;
    p.extent[0]       = (uint32_t)region.size.width;
    p.extent[1]       = (uint32_t)region.size.height;
    p.extent[2]       = (uint32_t)region.size.depth;
    p.level           = (uint32_t)level;
    p.slice           = (uint32_t)slice;
    p.bytes_per_row   = bpr;
    p.bytes_per_image = bpi;
    IvgCompletionRecord cmp{};
    if (_device.client->submit_control_pod(IVG_OP_TEXTURE_READBACK, &p, sizeof(p), cmp) && cmp.status == 0) {
        std::memcpy(pixelBytes, _device.client->region_base() + stg, total);
    }
}
@end

@implementation InfernoMTLLibrary
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h {
    if ((self = [super init])) { _device = dev; _handle = h; }
    return self;
}
- (id<MTLFunction>)newFunctionWithName:(NSString*)name {
    InfernoMTLFunction* fn = [[InfernoMTLFunction alloc] init];
    [fn setValue:@(_handle) forKey:@"libraryHandle"];
    [fn setValue:name forKey:@"name"];
    [fn setValue:_device forKey:@"device"];
    return (id<MTLFunction>)fn;
}
@end

@implementation InfernoMTLFunction
@end

@implementation InfernoMTLComputePipelineState
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h {
    if ((self = [super init])) { _device = dev; _handle = h; }
    return self;
}
@end

@implementation InfernoMTLRenderPipelineState
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h {
    if ((self = [super init])) { _device = dev; _handle = h; }
    return self;
}
@end

@implementation InfernoMTLCommandQueue {
    WireRing _subRing;
    WireRing _cmpRing;
}
- (instancetype)initWithDevice:(InfernoMTLDevice*)dev handle:(uint64_t)h sub:(WireRing)s cmp:(WireRing)c {
    if ((self = [super init])) { _device = dev; _handle = h; _subRing = s; _cmpRing = c; }
    return self;
}
- (WireRing&)subRing { return _subRing; }
- (WireRing&)cmpRing { return _cmpRing; }
- (id<MTLCommandBuffer>)commandBuffer {
    return (id<MTLCommandBuffer>)[[InfernoMTLCommandBuffer alloc] initWithQueue:self];
}
@end

@implementation InfernoMTLCommandBuffer {
    NSMutableArray<void (^)(id<MTLCommandBuffer>)>* _completedHandlers;
    std::atomic<uint64_t> _seq{1};
}
- (instancetype)initWithQueue:(InfernoMTLCommandQueue*)q {
    if ((self = [super init])) {
        _queue = q;
        _handle = q.device.client->alloc_handle();
        _completedHandlers = [NSMutableArray array];
        IvgCmdBufferBeginPayload p{_handle};
        [self appendRecord:IVG_QOP_CMD_BUFFER_BEGIN payload:&p len:sizeof(p) tail:nullptr tailLen:0];
    }
    return self;
}
- (void)appendRecord:(uint16_t)opcode payload:(const void*)payload len:(size_t)len tail:(const void*)tail tailLen:(size_t)tailLen {
    IvgRecordHeader hdr{};
    hdr.opcode     = opcode;
    hdr.size       = static_cast<uint32_t>(sizeof(hdr) + len + tailLen);
    hdr.context_id = _queue.device.client->context_id();
    hdr.seqno      = _seq.fetch_add(1, std::memory_order_relaxed);
    std::vector<uint8_t> pkt(hdr.size);
    std::memcpy(pkt.data(), &hdr, sizeof(hdr));
    if (payload && len > 0) std::memcpy(pkt.data() + sizeof(hdr), payload, len);
    if (tail && tailLen > 0) std::memcpy(pkt.data() + sizeof(hdr) + len, tail, tailLen);
    [_queue subRing].push(pkt.data(), pkt.size());
}
- (void)addCompletedHandler:(void (^)(id<MTLCommandBuffer>))block {
    [_completedHandlers addObject:[block copy]];
}
- (id<MTLComputeCommandEncoder>)computeCommandEncoder {
    return (id<MTLComputeCommandEncoder>)[[InfernoMTLComputeCommandEncoder alloc] initWithCommandBuffer:self];
}
- (id<MTLRenderCommandEncoder>)renderCommandEncoderWithDescriptor:(MTLRenderPassDescriptor*)desc {
    return (id<MTLRenderCommandEncoder>)[[InfernoMTLRenderCommandEncoder alloc] initWithCommandBuffer:self descriptor:desc];
}
- (id<MTLBlitCommandEncoder>)blitCommandEncoder {
    return (id<MTLBlitCommandEncoder>)[[InfernoMTLBlitCommandEncoder alloc] initWithCommandBuffer:self];
}
- (void)commit {
    IvgCmdBufferCommitPayload p{_handle, 1, 0};
    [self appendRecord:IVG_QOP_CMD_BUFFER_COMMIT payload:&p len:sizeof(p) tail:nullptr tailLen:0];
    if ([_queue subRing].host_idle()) {
        _queue.device.client->ring_doorbell(_queue.handle);
    }
}
- (void)waitUntilCompleted {
    std::vector<uint8_t> buf;
    while (![_queue cmpRing].pop(buf)) { sched_yield(); }
    for (void (^blk)(id<MTLCommandBuffer>) in _completedHandlers) blk((id<MTLCommandBuffer>)self);
}
@end

@implementation InfernoMTLComputeCommandEncoder {
    __weak InfernoMTLCommandBuffer* _cb;
}
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf {
    if ((self = [super init])) {
        _cb = cmdBuf;
        [_cb appendRecord:IVG_QOP_COMPUTE_BEGIN payload:nullptr len:0 tail:nullptr tailLen:0];
    }
    return self;
}
- (void)setComputePipelineState:(id<MTLComputePipelineState>)state {
    InfernoMTLComputePipelineState* s = (InfernoMTLComputePipelineState*)state;
    IvgComputeSetPipelinePayload p{s.handle};
    [_cb appendRecord:IVG_QOP_COMPUTE_SET_PIPELINE payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setBuffer:(id<MTLBuffer>)buffer offset:(NSUInteger)offset atIndex:(NSUInteger)index {
    InfernoMTLBuffer* b = (InfernoMTLBuffer*)buffer;
    if (b) [b didModifyRange:NSMakeRange(0, b.length)];
    IvgComputeSetBufferPayload p{b ? b.handle : 0, offset, (uint32_t)index, 0};
    [_cb appendRecord:IVG_QOP_COMPUTE_SET_BUFFER payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setTexture:(id<MTLTexture>)texture atIndex:(NSUInteger)index {
    InfernoMTLTexture* t = (InfernoMTLTexture*)texture;
    IvgComputeSetTexturePayload p{t ? t.handle : 0, (uint32_t)index, 0};
    [_cb appendRecord:IVG_QOP_COMPUTE_SET_TEXTURE payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setBytes:(const void*)bytes length:(NSUInteger)length atIndex:(NSUInteger)index {
    IvgSetBytesPayload p{(uint32_t)index, (uint32_t)length};
    [_cb appendRecord:IVG_QOP_COMPUTE_SET_BYTES payload:&p len:sizeof(p) tail:bytes tailLen:length];
}
- (void)dispatchThreads:(MTLSize)threads threadsPerThreadgroup:(MTLSize)tg {
    IvgComputeDispatchPayload p{{(uint32_t)threads.width, (uint32_t)threads.height, (uint32_t)threads.depth},
                                {(uint32_t)tg.width, (uint32_t)tg.height, (uint32_t)tg.depth}};
    [_cb appendRecord:IVG_QOP_COMPUTE_DISPATCH_THREADS payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)dispatchThreadgroups:(MTLSize)groups threadsPerThreadgroup:(MTLSize)tg {
    IvgComputeDispatchPayload p{{(uint32_t)groups.width, (uint32_t)groups.height, (uint32_t)groups.depth},
                                {(uint32_t)tg.width, (uint32_t)tg.height, (uint32_t)tg.depth}};
    [_cb appendRecord:IVG_QOP_COMPUTE_DISPATCH_THREADGROUPS payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)endEncoding {
    [_cb appendRecord:IVG_QOP_COMPUTE_END payload:nullptr len:0 tail:nullptr tailLen:0];
}
@end

@implementation InfernoMTLRenderCommandEncoder {
    __weak InfernoMTLCommandBuffer* _cb;
}
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf descriptor:(MTLRenderPassDescriptor*)desc {
    if ((self = [super init])) {
        _cb = cmdBuf;
        IvgRenderBeginPassPayload p{};
        p.render_target_width  = (uint32_t)desc.renderTargetWidth;
        p.render_target_height = (uint32_t)desc.renderTargetHeight;
        for (NSUInteger i = 0; i < 8; ++i) {
            MTLRenderPassColorAttachmentDescriptor* ca = desc.colorAttachments[i];
            if (ca.texture) {
                InfernoMTLTexture* t = (InfernoMTLTexture*)ca.texture;
                p.color_attachments[i].texture_id     = t.handle;
                p.color_attachments[i].clear_color[0] = ca.clearColor.red;
                p.color_attachments[i].clear_color[1] = ca.clearColor.green;
                p.color_attachments[i].clear_color[2] = ca.clearColor.blue;
                p.color_attachments[i].clear_color[3] = ca.clearColor.alpha;
                p.color_attachments[i].level          = (uint32_t)ca.level;
                p.color_attachments[i].slice          = (uint32_t)ca.slice;
                p.color_attachments[i].load_action    = (uint32_t)ca.loadAction;
                p.color_attachments[i].store_action   = (uint32_t)ca.storeAction;
            }
        }
        [_cb appendRecord:IVG_QOP_RENDER_BEGIN_PASS payload:&p len:sizeof(p) tail:nullptr tailLen:0];
    }
    return self;
}
- (void)setRenderPipelineState:(id<MTLRenderPipelineState>)state {
    InfernoMTLRenderPipelineState* s = (InfernoMTLRenderPipelineState*)state;
    IvgRenderSetPipelinePayload p{s.handle};
    [_cb appendRecord:IVG_QOP_RENDER_SET_PIPELINE payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setViewport:(MTLViewport)vp {
    IvgRenderSetViewportPayload p{vp.originX, vp.originY, vp.width, vp.height, vp.znear, vp.zfar};
    [_cb appendRecord:IVG_QOP_RENDER_SET_VIEWPORT payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setScissorRect:(MTLScissorRect)sc {
    IvgRenderSetScissorPayload p{(uint32_t)sc.x, (uint32_t)sc.y, (uint32_t)sc.width, (uint32_t)sc.height};
    [_cb appendRecord:IVG_QOP_RENDER_SET_SCISSOR payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setVertexBuffer:(id<MTLBuffer>)buf offset:(NSUInteger)off atIndex:(NSUInteger)idx {
    InfernoMTLBuffer* b = (InfernoMTLBuffer*)buf;
    if (b) [b didModifyRange:NSMakeRange(0, b.length)];
    IvgRenderSetBufferPayload p{b ? b.handle : 0, off, (uint32_t)idx, 0};
    [_cb appendRecord:IVG_QOP_RENDER_SET_VERTEX_BUFFER payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setFragmentBuffer:(id<MTLBuffer>)buf offset:(NSUInteger)off atIndex:(NSUInteger)idx {
    InfernoMTLBuffer* b = (InfernoMTLBuffer*)buf;
    if (b) [b didModifyRange:NSMakeRange(0, b.length)];
    IvgRenderSetBufferPayload p{b ? b.handle : 0, off, (uint32_t)idx, 0};
    [_cb appendRecord:IVG_QOP_RENDER_SET_FRAGMENT_BUFFER payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)setFragmentTexture:(id<MTLTexture>)tex atIndex:(NSUInteger)idx {
    InfernoMTLTexture* t = (InfernoMTLTexture*)tex;
    IvgRenderSetTexturePayload p{t ? t.handle : 0, (uint32_t)idx, 0};
    [_cb appendRecord:IVG_QOP_RENDER_SET_FRAGMENT_TEXTURE payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)drawPrimitives:(MTLPrimitiveType)type vertexStart:(NSUInteger)start vertexCount:(NSUInteger)count {
    IvgRenderDrawPrimitivesPayload p{(uint32_t)type, (uint32_t)start, (uint32_t)count, 1, 0, 0};
    [_cb appendRecord:IVG_QOP_RENDER_DRAW_PRIMITIVES payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)endEncoding {
    [_cb appendRecord:IVG_QOP_RENDER_END_PASS payload:nullptr len:0 tail:nullptr tailLen:0];
}
@end

@implementation InfernoMTLBlitCommandEncoder {
    __weak InfernoMTLCommandBuffer* _cb;
}
- (instancetype)initWithCommandBuffer:(InfernoMTLCommandBuffer*)cmdBuf {
    if ((self = [super init])) {
        _cb = cmdBuf;
        [_cb appendRecord:IVG_QOP_BLIT_BEGIN payload:nullptr len:0 tail:nullptr tailLen:0];
    }
    return self;
}
- (void)fillBuffer:(id<MTLBuffer>)buf range:(NSRange)range value:(uint8_t)val {
    InfernoMTLBuffer* b = (InfernoMTLBuffer*)buf;
    IvgBlitFillBufferPayload p{b.handle, range.location, range.length, val, {0}};
    [_cb appendRecord:IVG_QOP_BLIT_FILL_BUFFER payload:&p len:sizeof(p) tail:nullptr tailLen:0];
}
- (void)endEncoding {
    [_cb appendRecord:IVG_QOP_BLIT_END payload:nullptr len:0 tail:nullptr tailLen:0];
}
@end

@implementation InfernoMTLDevice {
    IvgDeviceCaps _caps;
}
- (instancetype)init {
    if ((self = [super init])) {
        auto& tr = GuestTransport::instance();
        if (tr.client) {
            size_t stg = tr.client->alloc_staging(sizeof(IvgDeviceCaps));
            tr.client->get_device_caps(stg, _caps);
        }
    }
    return self;
}
- (WireClient*)client { return GuestTransport::instance().client.get(); }
- (NSString*)name { return [NSString stringWithUTF8String:(const char*)_caps.device_name]; }
- (BOOL)hasUnifiedMemory { return _caps.has_unified_memory != 0; }
- (NSUInteger)maxThreadgroupMemoryLength { return _caps.max_threadgroup_memory_length; }

- (id<MTLCommandQueue>)newCommandQueue {
    uint64_t qid = self.client->alloc_handle();
    size_t   sub_hdr  = self.client->alloc_staging(64);
    size_t   sub_data = self.client->alloc_staging(16384);
    size_t   cmp_hdr  = self.client->alloc_staging(64);
    size_t   cmp_data = self.client->alloc_staging(16384);
    WireRing s, c;
    self.client->create_queue(qid, sub_hdr, sub_data, cmp_hdr, cmp_data, 16384, s, c);
    return (id<MTLCommandQueue>)[[InfernoMTLCommandQueue alloc] initWithDevice:self handle:qid sub:s cmp:c];
}

- (id<MTLBuffer>)newBufferWithLength:(NSUInteger)length options:(MTLResourceOptions)options {
    uint64_t blk = self.client->alloc_handle();
    uint64_t buf = self.client->alloc_handle();
    IvgBlockAllocPayload ba{blk, length, 0, 0};
    IvgCompletionRecord  cmp{};
    self.client->submit_control_pod(IVG_OP_BLOCK_ALLOC, &ba, sizeof(ba), cmp);
    IvgBufferCreatePayload bc{buf, blk, 0, length, 0};
    self.client->submit_control_pod(IVG_OP_BUFFER_CREATE, &bc, sizeof(bc), cmp);
    return (id<MTLBuffer>)[[InfernoMTLBuffer alloc] initWithDevice:self handle:buf block:blk length:length options:options];
}

- (id<MTLTexture>)newTextureWithDescriptor:(MTLTextureDescriptor*)desc {
    uint64_t tid = self.client->alloc_handle();
    IvgTextureCreatePayload p{};
    p.texture_id              = tid;
    p.desc.texture_type       = (uint32_t)desc.textureType;
    p.desc.pixel_format       = (uint32_t)desc.pixelFormat;
    p.desc.width              = (uint32_t)desc.width;
    p.desc.height             = (uint32_t)desc.height;
    p.desc.depth              = (uint32_t)desc.depth;
    p.desc.mipmap_level_count = (uint32_t)desc.mipmapLevelCount;
    p.desc.sample_count       = (uint32_t)desc.sampleCount;
    p.desc.array_length       = (uint32_t)desc.arrayLength;
    p.desc.usage              = (uint32_t)desc.usage;
    p.desc.storage_mode       = (uint32_t)desc.storageMode;
    IvgCompletionRecord cmp{};
    self.client->submit_control_pod(IVG_OP_TEXTURE_CREATE, &p, sizeof(p), cmp);
    return (id<MTLTexture>)[[InfernoMTLTexture alloc] initWithDevice:self handle:tid desc:desc];
}

- (id<MTLTexture>)newTextureWithDescriptor:(MTLTextureDescriptor*)desc iosurface:(IOSurfaceRef)iosurface plane:(NSUInteger)plane {
    uint64_t tid = self.client->alloc_handle();
    self.client->surface_import(
        IOSurfaceGetID(iosurface),
        tid,
        (uint32_t)desc.width,
        (uint32_t)desc.height,
        (uint32_t)desc.pixelFormat,
        IOSurfaceGetBytesPerRowOfPlane(iosurface, plane));
    return (id<MTLTexture>)[[InfernoMTLTexture alloc] initWithDevice:self handle:tid desc:desc];
}

- (BOOL)presentSurfaceTexture:(id<MTLTexture>)texture {
    InfernoMTLTexture* t = (InfernoMTLTexture*)texture;
    return self.client->display_present(t.handle, 0, 0) ? YES : NO;
}

- (id<MTLLibrary>)newLibraryWithData:(dispatch_data_t)data error:(NSError**)error {
    size_t size = dispatch_data_get_size(data);
    size_t stg  = self.client->alloc_staging(size);
    const void* ptr = nullptr;
    size_t mapped_len = 0;
    dispatch_data_t map = dispatch_data_create_map(data, &ptr, &mapped_len);
    std::memcpy(self.client->region_base() + stg, ptr, size);
    (void)map;

    uint64_t lid = self.client->alloc_handle();
    IvgLibraryCreatePayload p{};
    p.library_id    = lid;
    p.region_offset = stg;
    p.byte_len      = size;
    WireClient::sha256(static_cast<const uint8_t*>(ptr), size, p.sha256);
    IvgCompletionRecord cmp{};
    if (!self.client->submit_control_pod(IVG_OP_LIBRARY_CREATE, &p, sizeof(p), cmp) || cmp.status != 0) {
        if (error) *error = [NSError errorWithDomain:@"InfernoMetal" code:cmp.status userInfo:nil];
        return nil;
    }
    return (id<MTLLibrary>)[[InfernoMTLLibrary alloc] initWithDevice:self handle:lid];
}

- (id<MTLComputePipelineState>)newComputePipelineStateWithFunction:(id<MTLFunction>)fn error:(NSError**)error {
    InfernoMTLFunction* f = (InfernoMTLFunction*)fn;
    const char* cname = f.name.UTF8String;
    size_t nlen = std::strlen(cname);
    uint64_t pid = self.client->alloc_handle();
    IvgComputePipelineCreatePayload p{};
    p.pipeline_id       = pid;
    p.library_id        = f.libraryHandle;
    p.function_name_len = (uint32_t)nlen;
    IvgCompletionRecord cmp{};
    if (!self.client->submit_control_with_tail(IVG_OP_PIPELINE_CREATE_COMPUTE, &p, sizeof(p), cname, nlen, cmp)
        || cmp.status != 0)
    {
        if (error) *error = [NSError errorWithDomain:@"InfernoMetal" code:cmp.status userInfo:nil];
        return nil;
    }
    return (id<MTLComputePipelineState>)[[InfernoMTLComputePipelineState alloc] initWithDevice:self handle:pid];
}

- (id<MTLRenderPipelineState>)newRenderPipelineStateWithDescriptor:(MTLRenderPipelineDescriptor*)desc error:(NSError**)error {
    InfernoMTLFunction* vf = (InfernoMTLFunction*)desc.vertexFunction;
    InfernoMTLFunction* ff = (InfernoMTLFunction*)desc.fragmentFunction;
    const char* vname = vf.name.UTF8String;
    const char* fname = ff ? ff.name.UTF8String : "";
    size_t vlen = std::strlen(vname);
    size_t flen = std::strlen(fname);

    uint64_t pid = self.client->alloc_handle();
    IvgRenderPipelineCreatePayload p{};
    p.pipeline_id           = pid;
    p.vertex_library_id     = vf.libraryHandle;
    p.fragment_library_id   = ff ? ff.libraryHandle : 0;
    p.vertex_name_len       = (uint32_t)vlen;
    p.fragment_name_len     = (uint32_t)flen;
    p.depth_pixel_format    = (uint32_t)desc.depthAttachmentPixelFormat;
    p.stencil_pixel_format  = (uint32_t)desc.stencilAttachmentPixelFormat;
    p.sample_count          = (uint32_t)desc.rasterSampleCount;
    p.rasterization_enabled = desc.isRasterizationEnabled ? 1 : 0;
    for (NSUInteger i = 0; i < 8; ++i) {
        MTLRenderPipelineColorAttachmentDescriptor* ca = desc.colorAttachments[i];
        p.color_attachments[i].pixel_format     = (uint32_t)ca.pixelFormat;
        p.color_attachments[i].blending_enabled = ca.isBlendingEnabled ? 1 : 0;
        p.color_attachments[i].write_mask       = (uint8_t)ca.writeMask;
    }

    std::vector<uint8_t> names(vlen + flen);
    std::memcpy(names.data(), vname, vlen);
    if (flen > 0) std::memcpy(names.data() + vlen, fname, flen);

    IvgCompletionRecord cmp{};
    if (!self.client->submit_control_with_tail(IVG_OP_PIPELINE_CREATE_RENDER, &p, sizeof(p), names.data(), names.size(), cmp)
        || cmp.status != 0)
    {
        if (error) *error = [NSError errorWithDomain:@"InfernoMetal" code:cmp.status userInfo:nil];
        return nil;
    }
    return (id<MTLRenderPipelineState>)[[InfernoMTLRenderPipelineState alloc] initWithDevice:self handle:pid];
}

- (NSMethodSignature*)methodSignatureForSelector:(SEL)sel {
    LOG_SPI(@"unhandled selector on InfernoMTLDevice: %@", NSStringFromSelector(sel));
    return [super methodSignatureForSelector:sel];
}
@end

extern "C" {

id<MTLDevice> InfernoMTLCreateSystemDefaultDevice(void) {
    static InfernoMTLDevice* dev = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        if (GuestTransport::instance().client) {
            dev = [[InfernoMTLDevice alloc] init];
        }
    });
    return (id<MTLDevice>)dev;
}

NSArray<id<MTLDevice>>* InfernoMTLCopyAllDevices(void) {
    id<MTLDevice> d = InfernoMTLCreateSystemDefaultDevice();
    return d ? @[d] : @[];
}

}
