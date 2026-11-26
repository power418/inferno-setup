#pragma once

#ifdef INFERNO_FREESTANDING_IOS14
#include "../include/ios14_iokit_abi.hpp"

struct InfernoVgpuFreestandingState {
    void*    provider;
    void*    regsMem;
    void*    dbMem;
    void*    regionMem;
    void*    regsMap;
    void*    dbMap;
    uint64_t regsVa;
    uint64_t dbVa;
    uint32_t lastError;
    bool     deviceLost;
    bool     hostReady;
};

extern "C" {
bool InfernoVgpuService_probe(InfernoVgpuFreestandingState* state, void* provider);
void InfernoVgpuService_stop(InfernoVgpuFreestandingState* state);
void InfernoVgpuService_ringDoorbell(InfernoVgpuFreestandingState* state, uint64_t queueId);
IOReturn InfernoVgpuUserClient_externalMethod(InfernoVgpuFreestandingState* state,
                                            uint32_t selector,
                                            InfernoIOExternalMethodArguments* args);
}

#else

#include <IOKit/IOService.h>
#include <IOKit/IOUserClient.h>
#include <IOKit/IOBufferMemoryDescriptor.h>

class InfernoVgpuService : public IOService {
    OSDeclareDefaultStructors(InfernoVgpuService)

public:
    bool start(IOService* provider) override;
    void stop(IOService* provider) override;

    IOMemoryDescriptor* regionMemory() const { return regionMem_; }
    void ringDoorbell(uint64_t queueId);
    bool isDeviceLost() const;
    uint32_t readReg(uint32_t offset) const;

private:
    bool pingControlRing();

    IOMemoryDescriptor* regsMem_   = nullptr;
    IOMemoryDescriptor* dbMem_     = nullptr;
    IOMemoryDescriptor* regionMem_ = nullptr;
    IOMemoryMap*        regsMap_   = nullptr;
    IOMemoryMap*        dbMap_     = nullptr;
    IOMemoryMap*        regionMap_ = nullptr;
};

class InfernoVgpuUserClient : public IOUserClient {
    OSDeclareDefaultStructors(InfernoVgpuUserClient)

public:
    bool initWithTask(task_t owningTask, void* securityToken, UInt32 type, OSDictionary* properties) override;
    bool start(IOService* provider) override;
    IOReturn clientMemoryForType(UInt32 type, IOOptionBits* options, IOMemoryDescriptor** memory) override;
    IOReturn externalMethod(uint32_t selector, IOExternalMethodArguments* arguments,
                            IOExternalMethodDispatch* dispatch, OSObject* target, void* reference) override;

private:
    InfernoVgpuService* service_ = nullptr;
};

#endif
