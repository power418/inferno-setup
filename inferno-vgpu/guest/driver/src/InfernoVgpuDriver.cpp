#include "InfernoVgpuDriver.hpp"
#include "inferno_graphics_wire.h"
#include <IOKit/IOLib.h>

#define super IOService
OSDefineMetaClassAndStructors(InfernoVgpuService, IOService)

bool InfernoVgpuService::start(IOService* provider) {
    if (!super::start(provider)) return false;

    /* Region 0 = Control MMIO, Region 1 = Doorbell MMIO, Region 2 = Shared Region RAM */
    regsMem_   = provider->getDeviceMemoryWithIndex(0);
    dbMem_     = provider->getDeviceMemoryWithIndex(1);
    regionMem_ = provider->getDeviceMemoryWithIndex(2);
    if (!regsMem_ || !dbMem_ || !regionMem_) {
        IOLog("InfernoVgpuService: missing device memory ranges\n");
        return false;
    }

    regsMap_ = regsMem_->map();
    dbMap_   = dbMem_->map();
    if (!regsMap_ || !dbMap_) return false;

    volatile uint32_t* regs = reinterpret_cast<volatile uint32_t*>(regsMap_->getVirtualAddress());
    if (regs[IVG_REG_MAGIC / 4] != IVG_MAGIC) {
        IOLog("InfernoVgpuService: bad magic 0x%x\n", regs[IVG_REG_MAGIC / 4]);
        return false;
    }

    registerService();
    return true;
}

void InfernoVgpuService::stop(IOService* provider) {
    OSSafeReleaseNULL(regsMap_);
    OSSafeReleaseNULL(dbMap_);
    super::stop(provider);
}

void InfernoVgpuService::ringDoorbell(uint64_t queueId) {
    if (!dbMap_) return;
    volatile uint32_t* db = reinterpret_cast<volatile uint32_t*>(dbMap_->getVirtualAddress());
    db[0] = static_cast<uint32_t>(queueId);
}

#undef super
#define super IOUserClient
OSDefineMetaClassAndStructors(InfernoVgpuUserClient, IOUserClient)

bool InfernoVgpuUserClient::initWithTask(task_t owningTask, void* securityToken, UInt32 type, OSDictionary* properties) {
    return super::initWithTask(owningTask, securityToken, type, properties);
}

bool InfernoVgpuUserClient::start(IOService* provider) {
    service_ = OSDynamicCast(InfernoVgpuService, provider);
    return service_ && super::start(provider);
}

IOReturn InfernoVgpuUserClient::clientMemoryForType(UInt32 type, IOOptionBits* options, IOMemoryDescriptor** memory) {
    if (!service_ || type != 0) return kIOReturnBadArgument;
    IOMemoryDescriptor* mem = service_->regionMemory();
    if (!mem) return kIOReturnNotReady;
    mem->retain();
    *memory  = mem;
    *options = kIOMapWriteCombineCache;
    return kIOReturnSuccess;
}

IOReturn InfernoVgpuUserClient::externalMethod(uint32_t selector, IOExternalMethodArguments* arguments,
                                               IOExternalMethodDispatch* dispatch, OSObject* target, void* reference) {
    (void)dispatch;
    (void)target;
    (void)reference;
    if (selector == 0 && arguments->scalarInputCount >= 1) {
        service_->ringDoorbell(arguments->scalarInput[0]);
        return kIOReturnSuccess;
    }
    return kIOReturnUnsupported;
}
