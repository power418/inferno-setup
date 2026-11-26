#include "InfernoVgpuDriver.hpp"
#include "inferno_graphics_wire.h"

#ifdef INFERNO_FREESTANDING_IOS14

namespace {

InfernoVgpuFreestandingState g_state{};

inline void barrier_dmb() {
    __asm__ __volatile__("dmb ish" ::: "memory");
}

inline bool probe_control_ping(volatile uint32_t* regs,
                               volatile uint32_t* db,
                               uint8_t* region_base,
                               size_t region_size) {
    if (!region_base || region_size < 0x20000u) {
        return false;
    }

    auto* sub_hdr = reinterpret_cast<volatile IvgRingHeader*>(region_base + 0x0);
    auto* cmp_hdr = reinterpret_cast<volatile IvgRingHeader*>(region_base + 0x10000);
    sub_hdr->producer      = 0;
    sub_hdr->consumer      = 0;
    sub_hdr->capacity_mask = 16384u - 1u;
    sub_hdr->flags         = IVG_RING_FLAG_HOST_IDLE;
    sub_hdr->data_offset   = 0x1000u;

    cmp_hdr->producer      = 0;
    cmp_hdr->consumer      = 0;
    cmp_hdr->capacity_mask = 16384u - 1u;
    cmp_hdr->flags         = IVG_RING_FLAG_HOST_IDLE;
    cmp_hdr->data_offset   = 0x11000u;
    barrier_dmb();

    regs[IVG_REG_CONTROL_SUB_OFF_LO / 4] = 0x0u;
    regs[IVG_REG_CONTROL_SUB_OFF_HI / 4] = 0x0u;
    regs[IVG_REG_CONTROL_CMP_OFF_LO / 4] = 0x10000u;
    regs[IVG_REG_CONTROL_CMP_OFF_HI / 4] = 0x0u;
    regs[IVG_REG_STATUS / 4]             = IVG_STATUS_DRIVER_READY;

    struct PingRec {
        IvgRecordHeader hdr;
        IvgPingPayload  payload;
    } pkt{};
    pkt.hdr.opcode     = IVG_OP_PING;
    pkt.hdr.size       = sizeof(PingRec);
    pkt.hdr.context_id = 0;
    pkt.hdr.seqno      = 1;
    pkt.payload.cookie = 0x1AFE2000ULL;

    const auto* src = reinterpret_cast<const uint8_t*>(&pkt);
    volatile uint8_t* dst = region_base + 0x1000u;
    for (size_t i = 0; i < sizeof(pkt); ++i) {
        dst[i] = src[i];
    }
    barrier_dmb();
    sub_hdr->producer = static_cast<uint32_t>((sizeof(pkt) + 7u) & ~7u);
    barrier_dmb();
    db[0] = 0;

    for (unsigned attempt = 0; attempt < 200u; ++attempt) {
        barrier_dmb();
        uint32_t prod = cmp_hdr->producer;
        uint32_t cons = cmp_hdr->consumer;
        if (prod - cons >= sizeof(IvgCompletionRecord)) {
            IvgCompletionRecord cmp{};
            auto* dst_cmp = reinterpret_cast<uint8_t*>(&cmp);
            const volatile uint8_t* csrc = region_base + 0x11000u + (cons & cmp_hdr->capacity_mask);
            for (size_t i = 0; i < sizeof(cmp); ++i) {
                dst_cmp[i] = csrc[i];
            }
            cmp_hdr->consumer = cons + static_cast<uint32_t>((sizeof(IvgCompletionRecord) + 7u) & ~7u);
            return cmp.seqno == 1 && cmp.result_u64 == 0x1AFE2000ULL;
        }
        IOSleep(1);
    }
    return false;
}

kern_return_t kmod_start_cb(kmod_info_t* /*ki*/, void* /*data*/) {
    IOLog("InfernoVgpuDriver: kmod loaded (iOS 14.0 arm64e)\n");
    return kIOReturnSuccess;
}

kern_return_t kmod_stop_cb(kmod_info_t* /*ki*/, void* /*data*/) {
    InfernoVgpuService_stop(&g_state);
    IOLog("InfernoVgpuDriver: kmod unloaded\n");
    return kIOReturnSuccess;
}

} // namespace

extern "C" {

__attribute__((visibility("default")))
kmod_info_t kmod_info = {
    nullptr,
    1, /* KMOD_INFO_VERSION */
    0,
    "dev.chefkiss.driver.InfernoVgpu",
    "1.0.0",
    -1,
    nullptr,
    0,
    0,
    0,
    kmod_start_cb,
    kmod_stop_cb,
};

bool InfernoVgpuService_probe(InfernoVgpuFreestandingState* state, void* provider) {
    if (!state || !provider) {
        return false;
    }
    state->provider  = provider;
    state->regsMem   = xnu_IOService_getDeviceMemoryWithIndex(provider, 0);
    state->dbMem     = xnu_IOService_getDeviceMemoryWithIndex(provider, 1);
    state->regionMem = xnu_IOService_getDeviceMemoryWithIndex(provider, 2);
    if (!state->regsMem || !state->dbMem || !state->regionMem) {
        IOLog("InfernoVgpuService: missing device memory ranges\n");
        return false;
    }

    state->regsMap = xnu_IOMemoryDescriptor_map(state->regsMem, 0);
    state->dbMap   = xnu_IOMemoryDescriptor_map(state->dbMem, 0);
    void* regMap   = xnu_IOMemoryDescriptor_map(state->regionMem, 0);
    if (!state->regsMap || !state->dbMap || !regMap) {
        IOLog("InfernoVgpuService: failed mapping MMIO/shared region\n");
        return false;
    }

    state->regsVa = xnu_IOMemoryMap_getVirtualAddress(state->regsMap);
    state->dbVa   = xnu_IOMemoryMap_getVirtualAddress(state->dbMap);
    uint64_t regVa  = xnu_IOMemoryMap_getVirtualAddress(regMap);
    uint64_t regLen = xnu_IOMemoryMap_getLength(regMap);

    volatile uint32_t* regs = reinterpret_cast<volatile uint32_t*>(state->regsVa);
    volatile uint32_t* db   = reinterpret_cast<volatile uint32_t*>(state->dbVa);
    uint32_t magic          = regs[IVG_REG_MAGIC / 4];
    if (magic != IVG_MAGIC) {
        IOLog("InfernoVgpuService: unexpected magic 0x%x\n", magic);
        xnu_OSObject_release(regMap);
        return false;
    }

    bool ping_ok = probe_control_ping(regs, db, reinterpret_cast<uint8_t*>(regVa), static_cast<size_t>(regLen));
    uint32_t status  = regs[IVG_REG_STATUS / 4];
    state->hostReady = (status & IVG_STATUS_HOST_READY) != 0;
    state->deviceLost = (status & IVG_STATUS_DEVICE_LOST) != 0;
    state->lastError  = regs[IVG_REG_ERROR_CODE / 4];
    xnu_OSObject_release(regMap);

    IOLog("InfernoVgpuService: attached to apple-pvgpu (status=0x%x ping=%s)\n",
          status, ping_ok ? "ok" : "timeout");
    return !state->deviceLost;
}

void InfernoVgpuService_stop(InfernoVgpuFreestandingState* state) {
    if (!state) return;
    if (state->regsMap) {
        xnu_OSObject_release(state->regsMap);
        state->regsMap = nullptr;
    }
    if (state->dbMap) {
        xnu_OSObject_release(state->dbMap);
        state->dbMap = nullptr;
    }
    state->regsVa = 0;
    state->dbVa   = 0;
}

void InfernoVgpuService_ringDoorbell(InfernoVgpuFreestandingState* state, uint64_t queueId) {
    if (!state || !state->dbVa || state->deviceLost) return;
    volatile uint32_t* db = reinterpret_cast<volatile uint32_t*>(state->dbVa);
    db[0]                 = static_cast<uint32_t>(queueId);
}

IOReturn InfernoVgpuUserClient_externalMethod(InfernoVgpuFreestandingState* state,
                                              uint32_t selector,
                                              InfernoIOExternalMethodArguments* args) {
    if (!state || !args) return kIOReturnBadArgument;
    if (state->deviceLost) return kIOReturnNotReady;

    switch (selector) {
        case 0: /* kDoorbellMethod */
            if (args->scalarInputCount < 1 || !args->scalarInput) return kIOReturnBadArgument;
            InfernoVgpuService_ringDoorbell(state, args->scalarInput[0]);
            return kIOReturnSuccess;
        case 1: /* kQueryStatusMethod */
            if (args->scalarOutputCount < 1 || !args->scalarOutput || !state->regsVa) return kIOReturnBadArgument;
            args->scalarOutput[0] = reinterpret_cast<volatile uint32_t*>(state->regsVa)[IVG_REG_STATUS / 4];
            return kIOReturnSuccess;
        case 2: /* kAckIrqMethod */
            if (args->scalarInputCount < 1 || !args->scalarInput || !state->regsVa) return kIOReturnBadArgument;
            reinterpret_cast<volatile uint32_t*>(state->regsVa)[IVG_REG_IRQ_ACK / 4] =
                static_cast<uint32_t>(args->scalarInput[0]);
            return kIOReturnSuccess;
        default:
            return kIOReturnUnsupported;
    }
}

} // extern "C"

#else

#include <IOKit/IOLib.h>

#define super IOService
OSDefineMetaClassAndStructors(InfernoVgpuService, IOService)

bool InfernoVgpuService::pingControlRing() {
    if (!regsMap_ || !dbMap_ || !regionMap_) return false;
    volatile uint32_t* regs = reinterpret_cast<volatile uint32_t*>(regsMap_->getVirtualAddress());
    volatile uint32_t* db   = reinterpret_cast<volatile uint32_t*>(dbMap_->getVirtualAddress());
    uint8_t* region_base    = reinterpret_cast<uint8_t*>(regionMap_->getVirtualAddress());
    size_t region_size      = static_cast<size_t>(regionMap_->getLength());
    if (!region_base || region_size < 0x20000u) return false;

    auto* sub_hdr = reinterpret_cast<volatile IvgRingHeader*>(region_base + 0x0);
    auto* cmp_hdr = reinterpret_cast<volatile IvgRingHeader*>(region_base + 0x10000);
    sub_hdr->producer      = 0;
    sub_hdr->consumer      = 0;
    sub_hdr->capacity_mask = 16384u - 1u;
    sub_hdr->flags         = IVG_RING_FLAG_HOST_IDLE;
    sub_hdr->data_offset   = 0x1000u;

    cmp_hdr->producer      = 0;
    cmp_hdr->consumer      = 0;
    cmp_hdr->capacity_mask = 16384u - 1u;
    cmp_hdr->flags         = IVG_RING_FLAG_HOST_IDLE;
    cmp_hdr->data_offset   = 0x11000u;
    __sync_synchronize();

    regs[IVG_REG_CONTROL_SUB_OFF_LO / 4] = 0x0u;
    regs[IVG_REG_CONTROL_SUB_OFF_HI / 4] = 0x0u;
    regs[IVG_REG_CONTROL_CMP_OFF_LO / 4] = 0x10000u;
    regs[IVG_REG_CONTROL_CMP_OFF_HI / 4] = 0x0u;
    regs[IVG_REG_STATUS / 4]             = IVG_STATUS_DRIVER_READY;

    struct PingRec {
        IvgRecordHeader hdr;
        IvgPingPayload  payload;
    } pkt{};
    pkt.hdr.opcode     = IVG_OP_PING;
    pkt.hdr.size       = sizeof(PingRec);
    pkt.hdr.context_id = 0;
    pkt.hdr.seqno      = 1;
    pkt.payload.cookie = 0x1AFE2000ULL;

    const auto* src       = reinterpret_cast<const uint8_t*>(&pkt);
    volatile uint8_t* dst = region_base + 0x1000u;
    for (size_t i = 0; i < sizeof(pkt); ++i) { dst[i] = src[i]; }
    __sync_synchronize();
    sub_hdr->producer = static_cast<uint32_t>((sizeof(pkt) + 7u) & ~7u);
    db[0]             = 0;

    for (unsigned attempt = 0; attempt < 200u; ++attempt) {
        __sync_synchronize();
        uint32_t prod = cmp_hdr->producer;
        uint32_t cons = cmp_hdr->consumer;
        if (prod - cons >= sizeof(IvgCompletionRecord)) {
            IvgCompletionRecord cmp{};
            auto* dst_cmp = reinterpret_cast<uint8_t*>(&cmp);
            const volatile uint8_t* csrc = region_base + 0x11000u + (cons & cmp_hdr->capacity_mask);
            for (size_t i = 0; i < sizeof(cmp); ++i) { dst_cmp[i] = csrc[i]; }
            cmp_hdr->consumer = cons + static_cast<uint32_t>((sizeof(IvgCompletionRecord) + 7u) & ~7u);
            return cmp.seqno == 1 && cmp.result_u64 == 0x1AFE2000ULL;
        }
        IOSleep(1);
    }
    return false;
}

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

    regsMap_   = regsMem_->map();
    dbMap_     = dbMem_->map();
    regionMap_ = regionMem_->map();
    if (!regsMap_ || !dbMap_ || !regionMap_) return false;

    volatile uint32_t* regs = reinterpret_cast<volatile uint32_t*>(regsMap_->getVirtualAddress());
    if (regs[IVG_REG_MAGIC / 4] != IVG_MAGIC) {
        IOLog("InfernoVgpuService: bad magic 0x%x\n", regs[IVG_REG_MAGIC / 4]);
        return false;
    }

    bool ping_ok = pingControlRing();
    IOLog("InfernoVgpuService: attached to apple-pvgpu (status=0x%x ping=%s)\n",
          regs[IVG_REG_STATUS / 4], ping_ok ? "ok" : "timeout");

    setProperty("IOMatchCategory", "IOAcceleratorES");
    setProperty("MetalPluginName", "InfernoMetal");
    setProperty("MetalPluginClassName", "InfernoMTLDevice");
    setProperty("IOGLESBundleName", "AppleMetalGLRenderer");
    registerService();
    return true;
}

void InfernoVgpuService::stop(IOService* provider) {
    OSSafeReleaseNULL(regionMap_);
    OSSafeReleaseNULL(regsMap_);
    OSSafeReleaseNULL(dbMap_);
    super::stop(provider);
}

void InfernoVgpuService::ringDoorbell(uint64_t queueId) {
    if (!dbMap_ || isDeviceLost()) return;
    volatile uint32_t* db = reinterpret_cast<volatile uint32_t*>(dbMap_->getVirtualAddress());
    db[0] = static_cast<uint32_t>(queueId);
}

bool InfernoVgpuService::isDeviceLost() const {
    return (readReg(IVG_REG_STATUS) & IVG_STATUS_DEVICE_LOST) != 0;
}

uint32_t InfernoVgpuService::readReg(uint32_t offset) const {
    if (!regsMap_) return 0;
    volatile uint32_t* regs = reinterpret_cast<volatile uint32_t*>(regsMap_->getVirtualAddress());
    return regs[offset / 4];
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
    if (service_->isDeviceLost()) return kIOReturnNotReady;
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
    if (!service_ || !arguments) return kIOReturnBadArgument;
    if (service_->isDeviceLost()) return kIOReturnNotReady;

    switch (selector) {
        case 0:
            if (arguments->scalarInputCount < 1) return kIOReturnBadArgument;
            service_->ringDoorbell(arguments->scalarInput[0]);
            return kIOReturnSuccess;
        case 1:
            if (arguments->scalarOutputCount < 1) return kIOReturnBadArgument;
            arguments->scalarOutput[0] = service_->readReg(IVG_REG_STATUS);
            return kIOReturnSuccess;
        default:
            return kIOReturnUnsupported;
    }
}

#endif
