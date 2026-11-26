#include "InfernoWireClient.hpp"
#include <cstdint>
#include <cstring>
#include <unistd.h>

#ifdef INFERNO_BUNDLE_FREESTANDING
void* operator new(size_t n) { return malloc(n); }
void  operator delete(void* p) noexcept { free(p); }
void  operator delete(void* p, size_t) noexcept { free(p); }
#endif

/*
 * Freestanding arm64e-apple-ios14.0 C++/IOKit entrypoint for `InfernoMetal.bundle`
 * when cross-compiled on Linux against extracted iOS 14.0 `.tbd` stubs.
 */

extern "C" {

typedef uint32_t mach_port_t;
typedef uint32_t io_object_t;
typedef io_object_t io_service_t;
typedef io_object_t io_connect_t;
typedef int32_t kern_return_t;

extern const mach_port_t kIOMasterPortDefault;
extern mach_port_t mach_task_self_(void);

void* IOServiceMatching(const char* name);
io_service_t IOServiceGetMatchingService(mach_port_t masterPort, void* matching);
kern_return_t IOServiceOpen(io_service_t service, mach_port_t owningTask, uint32_t type, io_connect_t* connect);
kern_return_t IOServiceClose(io_connect_t connect);
kern_return_t IOObjectRelease(io_object_t object);
kern_return_t IOConnectMapMemory(io_connect_t connect,
                                 uint32_t memoryType,
                                 mach_port_t intoTask,
                                 uint64_t* atAddress,
                                 uint64_t* ofSize,
                                 uint32_t options);
kern_return_t IOConnectCallScalarMethod(io_connect_t connect,
                                        uint32_t selector,
                                        const uint64_t* input,
                                        uint32_t inputCnt,
                                        uint64_t* output,
                                        uint32_t* outputCnt);

struct InfernoBundleTransport {
    io_connect_t conn{0};
    uint8_t*     region{nullptr};
    size_t       region_size{0};
    inferno::gpu::WireClient* client{nullptr};
};

__attribute__((visibility("default")))
uint32_t inferno_metal_bundle_version(void) {
    return IVG_VERSION;
}

__attribute__((visibility("default")))
InfernoBundleTransport* inferno_metal_create_transport(void) {
    io_service_t svc = IOServiceGetMatchingService(kIOMasterPortDefault, IOServiceMatching("InfernoVgpuService"));
    if (svc == 0) {
        return nullptr;
    }

    io_connect_t conn = 0;
    if (IOServiceOpen(svc, mach_task_self_(), 0, &conn) != 0) {
        IOObjectRelease(svc);
        return nullptr;
    }
    IOObjectRelease(svc);

    uint64_t addr = 0;
    uint64_t size = 0;
    if (IOConnectMapMemory(conn, 0, mach_task_self_(), &addr, &size, 1u /* kIOMapAnywhere */) != 0 || addr == 0) {
        IOServiceClose(conn);
        return nullptr;
    }

    auto* tr        = new InfernoBundleTransport();
    tr->conn        = conn;
    tr->region      = reinterpret_cast<uint8_t*>(addr);
    tr->region_size = static_cast<size_t>(size);
    uint32_t pid    = static_cast<uint32_t>(getpid());
    tr->client      = new inferno::gpu::WireClient(tr->region, tr->region_size, pid, [conn](uint64_t qid) {
        uint64_t input = qid;
        IOConnectCallScalarMethod(conn, 0, &input, 1, nullptr, nullptr);
    });
    if (!tr->client->initialize_control_rings(0x0, 0x1000, 0x10000, 0x11000, 16384)
        || !tr->client->create_context(pid))
    {
        delete tr->client;
        IOServiceClose(conn);
        delete tr;
        return nullptr;
    }

    IvgDeviceCaps caps{};
    size_t        caps_off = tr->client->alloc_staging(sizeof(IvgDeviceCaps));
    tr->client->get_device_caps(caps_off, caps);
    return tr;
}

__attribute__((visibility("default")))
int inferno_metal_ping(InfernoBundleTransport* tr, uint64_t cookie) {
    if (!tr || !tr->client) return -1;
    IvgCompletionRecord cmp{};
    if (!tr->client->ping(cookie, cmp) || cmp.status != 0 || cmp.result_u64 != cookie) {
        return -1;
    }
    return 0;
}

__attribute__((visibility("default")))
int inferno_metal_present_surface(InfernoBundleTransport* tr,
                                  uint32_t surface_id,
                                  uint32_t width,
                                  uint32_t height,
                                  uint32_t pixel_format,
                                  uint64_t bytes_per_row) {
    if (!tr || !tr->client) return -1;
    uint64_t tid = tr->client->alloc_handle();
    if (!tr->client->surface_import(surface_id, tid, width, height, pixel_format, bytes_per_row)) {
        return -1;
    }
    return tr->client->display_present(tid, 0, 0) ? 0 : -1;
}

} // extern "C"
