#pragma once

/*
 * Minimal freestanding XNU / IOKit ABI bindings verified against iOS 14.0 (18A5351d)
 * `kernelcache.research.iphone12b` (`inferno-vgpu-fw extract-abi`).
 */

#include <stdint.h>
#include <stddef.h>

typedef int32_t kern_return_t;
typedef kern_return_t IOReturn;
typedef uint32_t IOOptionBits;
typedef void* task_t;

#ifndef kIOReturnSuccess
#define kIOReturnSuccess     ((IOReturn)0)
#define kIOReturnError       ((IOReturn)0xe00002bc)
#define kIOReturnBadArgument ((IOReturn)0xe00002c2)
#define kIOReturnUnsupported ((IOReturn)0xe00002c7)
#define kIOReturnNotReady    ((IOReturn)0xe00002d8)
#endif

#ifndef kIOMapWriteCombineCache
#define kIOMapWriteCombineCache ((IOOptionBits)0x0400)
#endif

#define KMOD_MAX_NAME 64

typedef struct kmod_info_v1 {
    struct kmod_info_v1* next;
    int32_t              info_version;
    uint32_t             id;
    char                 name[KMOD_MAX_NAME];
    char                 version[KMOD_MAX_NAME];
    int32_t              reference_count;
    void*                reference_list;
    uint64_t             address;
    uint64_t             size;
    uint64_t             hdr_size;
    kern_return_t        (*start)(struct kmod_info_v1*, void*);
    kern_return_t        (*stop)(struct kmod_info_v1*, void*);
} kmod_info_t;

typedef struct {
    uint32_t version;
    uint32_t selector;
    const uint64_t* scalarInput;
    uint32_t scalarInputCount;
    const void* structureInput;
    uint32_t structureInputSize;
    void* structureInputDescriptor;
    uint64_t* scalarOutput;
    uint32_t scalarOutputCount;
    void* structureOutput;
    uint32_t structureOutputSize;
    void* structureOutputDescriptor;
} InfernoIOExternalMethodArguments;

extern "C" {
void IOLog(const char* format, ...) __asm("_IOLog");
void IOSleep(unsigned milliseconds) __asm("_IOSleep");

int xnu_IOService_start(void* self, void* provider) __asm("__ZN9IOService5startEPS_");
void xnu_IOService_stop(void* self, void* provider) __asm("__ZN9IOService4stopEPS_");
void xnu_IOService_registerService(void* self, IOOptionBits options) __asm("__ZN9IOService15registerServiceEj");
void* xnu_IOService_getDeviceMemoryWithIndex(void* self, uint32_t index) __asm("__ZN9IOService24getDeviceMemoryWithIndexEj");

void* xnu_IOMemoryDescriptor_map(void* self, IOOptionBits options) __asm("__ZN18IOMemoryDescriptor3mapEj");
uint64_t xnu_IOMemoryMap_getVirtualAddress(void* self) __asm("__ZN11IOMemoryMap17getVirtualAddressEv");
uint64_t xnu_IOMemoryMap_getLength(void* self) __asm("__ZN11IOMemoryMap9getLengthEv");

void xnu_OSObject_retain(const void* self) __asm("__ZNK8OSObject6retainEv");
void xnu_OSObject_release(const void* self) __asm("__ZNK8OSObject7releaseEv");

IOReturn xnu_IOGPU_start(void* self, void* provider) __asm("__ZN5IOGPU5startEP9IOService");
void xnu_IOGPU_stop(void* self, void* provider) __asm("__ZN5IOGPU4stopEP9IOService");
}
