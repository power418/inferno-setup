#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <cassert>
#include <cstdint>
#include <vector>

int main(int argc, char* argv[]) {
    @autoreleasepool {
        id<MTLDevice> dev = MTLCreateSystemDefaultDevice();
        if (!dev) {
            NSLog(@"No Metal device available");
            return 1;
        }
        NSLog(@"Metal device: %@ (unified=%d)", dev.name, dev.hasUnifiedMemory);

        NSError* err = nil;
        NSData* libData = [NSData dataWithContentsOfFile:@"shaders.metallib"];
        dispatch_data_t ddata = dispatch_data_create(libData.bytes, libData.length, nil, DISPATCH_DATA_DESTRUCTOR_DEFAULT);
        id<MTLLibrary> lib = [dev newLibraryWithData:ddata error:&err];
        assert(lib != nil && err == nil);

        id<MTLCommandQueue> queue = [dev newCommandQueue];
        assert(queue != nil);

        // Phase 2: Compute verification
        id<MTLFunction> addFn = [lib newFunctionWithName:@"vec_add"];
        id<MTLComputePipelineState> cps = [dev newComputePipelineStateWithFunction:addFn error:&err];
        assert(cps != nil);

        constexpr uint32_t N = 64;
        id<MTLBuffer> bufA = [dev newBufferWithLength:N * 4 options:MTLResourceStorageModeShared];
        id<MTLBuffer> bufB = [dev newBufferWithLength:N * 4 options:MTLResourceStorageModeShared];
        id<MTLBuffer> bufO = [dev newBufferWithLength:N * 4 options:MTLResourceStorageModeShared];
        auto* pa = static_cast<uint32_t*>(bufA.contents);
        auto* pb = static_cast<uint32_t*>(bufB.contents);
        for (uint32_t i = 0; i < N; ++i) { pa[i] = i; pb[i] = 1000 + i; }

        id<MTLCommandBuffer> cb = [queue commandBuffer];
        id<MTLComputeCommandEncoder> enc = [cb computeCommandEncoder];
        [enc setComputePipelineState:cps];
        [enc setBuffer:bufA offset:0 atIndex:0];
        [enc setBuffer:bufB offset:0 atIndex:1];
        [enc setBuffer:bufO offset:0 atIndex:2];
        [enc dispatchThreads:MTLSizeMake(N, 1, 1) threadsPerThreadgroup:MTLSizeMake(16, 1, 1)];
        [enc endEncoding];
        [cb commit];
        [cb waitUntilCompleted];

        // Phase 3: Offscreen triangle render + readback
        id<MTLFunction> vf = [lib newFunctionWithName:@"tri_vert"];
        id<MTLFunction> ff = [lib newFunctionWithName:@"tri_frag"];
        MTLRenderPipelineDescriptor* rdesc = [[MTLRenderPipelineDescriptor alloc] init];
        rdesc.vertexFunction = vf;
        rdesc.fragmentFunction = ff;
        rdesc.colorAttachments[0].pixelFormat = MTLPixelFormatBGRA8Unorm;
        id<MTLRenderPipelineState> rps = [dev newRenderPipelineStateWithDescriptor:rdesc error:&err];
        assert(rps != nil);

        MTLTextureDescriptor* tdesc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                                                         width:64 height:64 mipmapped:NO];
        tdesc.usage = MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead;
        id<MTLTexture> target = [dev newTextureWithDescriptor:tdesc];

        MTLRenderPassDescriptor* pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture     = target;
        pass.colorAttachments[0].loadAction  = MTLLoadActionClear;
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        pass.colorAttachments[0].clearColor  = MTLClearColorMake(0, 0, 0, 1);

        id<MTLCommandBuffer> rcb = [queue commandBuffer];
        id<MTLRenderCommandEncoder> renc = [rcb renderCommandEncoderWithDescriptor:pass];
        [renc setRenderPipelineState:rps];
        [renc drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
        [renc endEncoding];
        [rcb commit];
        [rcb waitUntilCompleted];

        std::vector<uint8_t> pixels(64 * 64 * 4, 0);
        [target getBytes:pixels.data() bytesPerRow:64 * 4 fromRegion:MTLRegionMake2D(0, 0, 64, 64) mipmapLevel:0];
        NSLog(@"Guest smoke test finished successfully.");
    }
    return 0;
}
