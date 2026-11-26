#pragma once

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

private:
    IOMemoryDescriptor* regsMem_   = nullptr;
    IOMemoryDescriptor* dbMem_     = nullptr;
    IOMemoryDescriptor* regionMem_ = nullptr;
    IOMemoryMap*        regsMap_   = nullptr;
    IOMemoryMap*        dbMap_     = nullptr;
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
