#pragma once

#import <Foundation/Foundation.h>
#import <Metal/Metal.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Entry points exposed for fspatcher / dyld interposition on iOS 14.
id<MTLDevice> InfernoMTLCreateSystemDefaultDevice(void);
NSArray<id<MTLDevice>>* InfernoMTLCopyAllDevices(void);

#ifdef __cplusplus
}
#endif
