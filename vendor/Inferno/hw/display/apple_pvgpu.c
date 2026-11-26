/*
 * Apple Paravirtualized Metal GPU device (`apple-pvgpu`) for ChefKiss Inferno.
 *
 * Bridges iOS guest Metal command rings to the out-of-process Rust
 * `inferno-vgpu-server` (powered by `metal2vulkan` + `metal2vulkan-runtime`),
 * isolating host Vulkan driver/compiler crashes from Inferno's SEP state.
 */

#include "qemu/osdep.h"
#include "hw/display/apple_pvgpu.h"
#include "hw/irq.h"
#include "hw/qdev-properties.h"
#include "hw/sysbus.h"
#include "qapi/error.h"
#include "qemu/error-report.h"
#include "qemu/lockable.h"
#include "qemu/main-loop.h"
#include "qemu/units.h"
#include "system/memory.h"

#include <errno.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

/* Matches crates/inferno-vgpu-graphics/include/inferno_graphics_wire.h */
#define IVG_MAGIC                0x49564750u
#define IVG_VERSION              ((1u << 16) | 0u)
#define IVG_REG_MAGIC            0x000u
#define IVG_REG_VERSION          0x004u
#define IVG_REG_STATUS           0x010u
#define IVG_REG_IRQ_STATUS       0x028u
#define IVG_REG_IRQ_ACK          0x02Cu
#define IVG_REG_REGION_SIZE_LO   0x030u
#define IVG_REG_REGION_SIZE_HI   0x034u
#define IVG_REG_DISPLAY_WIDTH    0x048u
#define IVG_REG_DISPLAY_HEIGHT   0x04Cu
#define IVG_REG_ERROR_CODE       0x050u

#define IVG_STATUS_DEVICE_LOST   (1u << 31)
#define IVG_IRQ_ERROR            (1u << 31)

/* Matches crates/inferno-vgpu-server/src/protocol.rs */
#define IVG_IPC_HELLO            1u
#define IVG_IPC_HELLO_ACK        2u
#define IVG_IPC_MMIO_READ        3u
#define IVG_IPC_MMIO_READ_ACK    4u
#define IVG_IPC_MMIO_WRITE       5u
#define IVG_IPC_ACK              6u
#define IVG_IPC_DOORBELL         7u
#define IVG_IPC_RESET            8u
#define IVG_IPC_IRQ_RAISE        11u
#define IVG_IPC_GET_CAPS         12u
#define IVG_IPC_GET_CAPS_ACK     13u

_Static_assert(sizeof(ApplePvgpuCaps) == 168, "ApplePvgpuCaps must match wire IvgDeviceCaps");

typedef struct QEMU_PACKED IvgIpcHeader {
    uint32_t msg_type;
    uint32_t arg0;
    uint64_t arg1;
    uint64_t arg2;
} IvgIpcHeader;

struct ApplePvgpuState {
    SysBusDevice parent_obj;

    MemoryRegion regs_mr;
    MemoryRegion doorbell_mr;
    MemoryRegion region_mr;
    qemu_irq     irq;

    char*    socket_path;
    uint64_t region_size;
    uint32_t width;
    uint32_t height;

    int       sock_fd;
    int       region_fd;
    bool      device_lost;
    bool      has_caps;
    ApplePvgpuCaps caps;
    uint32_t  pending_irq;
    QemuMutex io_lock;
};

static bool read_exact_fd(int fd, void* dst, size_t len)
{
    uint8_t* p   = dst;
    size_t   got = 0;
    while (got < len) {
        ssize_t n = recv(fd, p + got, len - got, 0);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return false;
        }
        if (n == 0) {
            return false;
        }
        got += (size_t)n;
    }
    return true;
}

static bool write_exact_fd(int fd, const void* src, size_t len)
{
    const uint8_t* p    = src;
    size_t         sent = 0;
    while (sent < len) {
        ssize_t n = send(fd, p + sent, len - sent, MSG_NOSIGNAL);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return false;
        }
        if (n == 0) {
            return false;
        }
        sent += (size_t)n;
    }
    return true;
}

static void apple_pvgpu_mark_lost(ApplePvgpuState* s, const char* reason)
{
    if (!s->device_lost) {
        warn_report("apple-pvgpu: %s — entering device-lost fallback (SEP remains alive)", reason);
    }
    s->device_lost = true;
    if (s->sock_fd >= 0) {
        qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL);
        close(s->sock_fd);
        s->sock_fd = -1;
    }
    s->pending_irq |= IVG_IRQ_ERROR;
    qemu_set_irq(s->irq, 1);
}

static void apple_pvgpu_sock_read(void* opaque)
{
    ApplePvgpuState* s = opaque;
    IvgIpcHeader     hdr;

    QEMU_LOCK_GUARD(&s->io_lock);
    if (s->sock_fd < 0) {
        return;
    }

    ssize_t n = recv(s->sock_fd, &hdr, sizeof(hdr), MSG_DONTWAIT);
    if (n == 0 || (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)) {
        apple_pvgpu_mark_lost(s, "GPU server disconnected");
        return;
    }
    if (n < (ssize_t)sizeof(hdr)) {
        if (n > 0) {
            uint8_t* p = (uint8_t*)&hdr;
            if (!read_exact_fd(s->sock_fd, p + n, sizeof(hdr) - (size_t)n)) {
                apple_pvgpu_mark_lost(s, "short IPC read from GPU server");
                return;
            }
        } else {
            return;
        }
    }

    if (hdr.msg_type == IVG_IPC_IRQ_RAISE) {
        s->pending_irq |= hdr.arg0;
        if (s->pending_irq != 0) {
            qemu_set_irq(s->irq, 1);
        }
    }
}

static bool apple_pvgpu_rpc_locked(ApplePvgpuState* s, const IvgIpcHeader* req, IvgIpcHeader* out_resp)
{
    if (s->sock_fd < 0 || s->device_lost) {
        return false;
    }
    if (!write_exact_fd(s->sock_fd, req, sizeof(*req))) {
        apple_pvgpu_mark_lost(s, "failed writing request to GPU server");
        return false;
    }
    for (;;) {
        IvgIpcHeader hdr;
        if (!read_exact_fd(s->sock_fd, &hdr, sizeof(hdr))) {
            apple_pvgpu_mark_lost(s, "failed reading response from GPU server");
            return false;
        }
        if (hdr.msg_type == IVG_IPC_IRQ_RAISE) {
            s->pending_irq |= hdr.arg0;
            if (s->pending_irq != 0) {
                qemu_set_irq(s->irq, 1);
            }
            continue;
        }
        *out_resp = hdr;
        return true;
    }
}

static uint64_t apple_pvgpu_regs_read(void* opaque, hwaddr offset, unsigned size)
{
    ApplePvgpuState* s = opaque;
    IvgIpcHeader     req = {
        .msg_type = IVG_IPC_MMIO_READ,
        .arg0     = 0,
        .arg1     = (uint64_t)offset,
        .arg2     = 0,
    };
    IvgIpcHeader resp;

    QEMU_LOCK_GUARD(&s->io_lock);
    if (apple_pvgpu_rpc_locked(s, &req, &resp) && resp.msg_type == IVG_IPC_MMIO_READ_ACK) {
        return resp.arg0;
    }

    switch (offset) {
        case IVG_REG_MAGIC: return IVG_MAGIC;
        case IVG_REG_VERSION: return IVG_VERSION;
        case IVG_REG_STATUS: return s->device_lost ? IVG_STATUS_DEVICE_LOST : 0;
        case IVG_REG_IRQ_STATUS: return s->pending_irq;
        case IVG_REG_REGION_SIZE_LO: return (uint32_t)s->region_size;
        case IVG_REG_REGION_SIZE_HI: return (uint32_t)(s->region_size >> 32);
        case IVG_REG_DISPLAY_WIDTH: return s->width;
        case IVG_REG_DISPLAY_HEIGHT: return s->height;
        case IVG_REG_ERROR_CODE: return s->device_lost ? 1u : 0u;
        default: return 0;
    }
}

static void apple_pvgpu_regs_write(void* opaque, hwaddr offset, uint64_t val, unsigned size)
{
    ApplePvgpuState* s = opaque;

    QEMU_LOCK_GUARD(&s->io_lock);
    if (offset == IVG_REG_IRQ_ACK) {
        s->pending_irq &= ~(uint32_t)val;
        if (s->pending_irq == 0) {
            qemu_set_irq(s->irq, 0);
        }
    }

    if (s->sock_fd >= 0 && !s->device_lost) {
        IvgIpcHeader req = {
            .msg_type = IVG_IPC_MMIO_WRITE,
            .arg0     = (uint32_t)val,
            .arg1     = (uint64_t)offset,
            .arg2     = 0,
        };
        IvgIpcHeader resp;
        (void)apple_pvgpu_rpc_locked(s, &req, &resp);
    }
}

static const MemoryRegionOps apple_pvgpu_regs_ops = {
    .read       = apple_pvgpu_regs_read,
    .write      = apple_pvgpu_regs_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .impl = {
        .min_access_size = 4,
        .max_access_size = 4,
    },
};

static uint64_t apple_pvgpu_db_read(void* opaque, hwaddr offset, unsigned size)
{
    (void)opaque;
    (void)offset;
    (void)size;
    return 0;
}

static void apple_pvgpu_db_write(void* opaque, hwaddr offset, uint64_t val, unsigned size)
{
    ApplePvgpuState* s = opaque;
    uint64_t         queue_id = (offset == 0) ? val : (offset >> 14);
    IvgIpcHeader     pkt = {
        .msg_type = IVG_IPC_DOORBELL,
        .arg0     = 0,
        .arg1     = queue_id,
        .arg2     = 0,
    };

    QEMU_LOCK_GUARD(&s->io_lock);
    if (s->sock_fd >= 0 && !s->device_lost) {
        if (!write_exact_fd(s->sock_fd, &pkt, sizeof(pkt))) {
            apple_pvgpu_mark_lost(s, "doorbell write failed");
        }
    }
}

static const MemoryRegionOps apple_pvgpu_db_ops = {
    .read       = apple_pvgpu_db_read,
    .write      = apple_pvgpu_db_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .impl = {
        .min_access_size = 4,
        .max_access_size = 4,
    },
};

static bool apple_pvgpu_connect(ApplePvgpuState* s)
{
    if (!s->socket_path || s->socket_path[0] == '\0') {
        return false;
    }

    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) {
        return false;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    if (strlen(s->socket_path) >= sizeof(addr.sun_path)) {
        close(fd);
        return false;
    }
    strcpy(addr.sun_path, s->socket_path);

    if (connect(fd, (struct sockaddr*)&addr, sizeof(addr)) != 0) {
        close(fd);
        return false;
    }

    IvgIpcHeader hello = {
        .msg_type = IVG_IPC_HELLO,
        .arg0     = 0,
        .arg1     = 0,
        .arg2     = 0,
    };
    if (!write_exact_fd(fd, &hello, sizeof(hello))) {
        close(fd);
        return false;
    }

    IvgIpcHeader ack;
    char         cmsg_buf[CMSG_SPACE(sizeof(int))];
    struct iovec iov = {
        .iov_base = &ack,
        .iov_len  = sizeof(ack),
    };
    struct msghdr msg = {
        .msg_iov        = &iov,
        .msg_iovlen     = 1,
        .msg_control    = cmsg_buf,
        .msg_controllen = sizeof(cmsg_buf),
    };

    ssize_t n = recvmsg(fd, &msg, MSG_WAITALL);
    if (n != (ssize_t)sizeof(ack) || ack.msg_type != IVG_IPC_HELLO_ACK) {
        close(fd);
        return false;
    }

    int mem_fd = -1;
    for (struct cmsghdr* cmsg = CMSG_FIRSTHDR(&msg); cmsg != NULL; cmsg = CMSG_NXTHDR(&msg, cmsg)) {
        if (cmsg->cmsg_level == SOL_SOCKET && cmsg->cmsg_type == SCM_RIGHTS) {
            memcpy(&mem_fd, CMSG_DATA(cmsg), sizeof(mem_fd));
            break;
        }
    }
    if (mem_fd < 0) {
        close(fd);
        return false;
    }

    s->sock_fd     = fd;
    s->region_fd   = mem_fd;
    s->region_size = ack.arg1;
    if (ack.arg2 != 0) {
        s->width  = (uint32_t)(ack.arg2 >> 32);
        s->height = (uint32_t)(ack.arg2 & 0xFFFFFFFFu);
    }

    IvgIpcHeader caps_req = {
        .msg_type = IVG_IPC_GET_CAPS,
        .arg0     = 0,
        .arg1     = 0,
        .arg2     = 0,
    };
    if (write_exact_fd(fd, &caps_req, sizeof(caps_req))) {
        IvgIpcHeader caps_ack;
        if (read_exact_fd(fd, &caps_ack, sizeof(caps_ack)) && caps_ack.msg_type == IVG_IPC_GET_CAPS_ACK
            && caps_ack.arg2 == sizeof(ApplePvgpuCaps))
        {
            if (read_exact_fd(fd, &s->caps, sizeof(s->caps))) {
                s->caps.device_name[sizeof(s->caps.device_name) - 1] = '\0';
                s->has_caps                                          = true;
            }
        }
    }
    return true;
}

bool apple_pvgpu_is_connected(const ApplePvgpuState* s)
{
    return s != NULL && s->sock_fd >= 0 && !s->device_lost;
}

uint64_t apple_pvgpu_region_size(const ApplePvgpuState* s)
{
    return s != NULL ? s->region_size : 0;
}

bool apple_pvgpu_get_caps(const ApplePvgpuState* s, ApplePvgpuCaps* out_caps)
{
    if (s == NULL || !s->has_caps || out_caps == NULL) {
        return false;
    }
    *out_caps = s->caps;
    return true;
}

static void apple_pvgpu_init(Object* obj)
{
    SysBusDevice*    sbd = SYS_BUS_DEVICE(obj);
    ApplePvgpuState* s   = APPLE_PVGPU(sbd);

    s->sock_fd   = -1;
    s->region_fd = -1;
    qemu_mutex_init(&s->io_lock);

    memory_region_init_io(&s->regs_mr, obj, &apple_pvgpu_regs_ops, s, TYPE_APPLE_PVGPU ".regs", 0x10000);
    sysbus_init_mmio(sbd, &s->regs_mr);

    memory_region_init_io(&s->doorbell_mr, obj, &apple_pvgpu_db_ops, s, TYPE_APPLE_PVGPU ".doorbell", 0x10000);
    sysbus_init_mmio(sbd, &s->doorbell_mr);

    sysbus_init_irq(sbd, &s->irq);
}

static void apple_pvgpu_realize(DeviceState* dev, Error** errp)
{
    SysBusDevice*    sbd = SYS_BUS_DEVICE(dev);
    ApplePvgpuState* s   = APPLE_PVGPU(dev);

    if (apple_pvgpu_connect(s)) {
        if (!memory_region_init_ram_from_fd(&s->region_mr, OBJECT(dev), TYPE_APPLE_PVGPU ".region",
                                            s->region_size, RAM_SHARED, s->region_fd, 0, errp))
        {
            return;
        }
        qemu_set_fd_handler(s->sock_fd, apple_pvgpu_sock_read, NULL, s);
        if (s->has_caps) {
            info_report("apple-pvgpu: connected to %s (gpu='%s', vendor=0x%04x, device=0x%04x, region=%" PRIu64 " MiB)",
                        s->socket_path, s->caps.device_name, s->caps.vendor_id, s->caps.device_id,
                        s->region_size / MiB);
        } else {
            info_report("apple-pvgpu: connected to %s (region=%" PRIu64 " MiB)",
                        s->socket_path, s->region_size / MiB);
        }
    } else {
        s->device_lost = true;
        memory_region_init_ram(&s->region_mr, OBJECT(dev), TYPE_APPLE_PVGPU ".region",
                               s->region_size, errp);
        warn_report("apple-pvgpu: could not connect to '%s'; running in device-lost fallback",
                    s->socket_path ? s->socket_path : "");
    }
    sysbus_init_mmio(sbd, &s->region_mr);
}

static void apple_pvgpu_reset_hold(Object* obj, ResetType type)
{
    ApplePvgpuState* s = APPLE_PVGPU(obj);
    (void)type;

    QEMU_LOCK_GUARD(&s->io_lock);
    s->pending_irq = 0;
    qemu_set_irq(s->irq, 0);
    if (s->sock_fd >= 0 && !s->device_lost) {
        IvgIpcHeader req = { .msg_type = IVG_IPC_RESET };
        IvgIpcHeader resp;
        (void)apple_pvgpu_rpc_locked(s, &req, &resp);
    }
}

static const Property apple_pvgpu_props[] = {
    DEFINE_PROP_STRING("socket-path", ApplePvgpuState, socket_path),
    DEFINE_PROP_UINT64("region-size", ApplePvgpuState, region_size, 64 * MiB),
    DEFINE_PROP_UINT32("width", ApplePvgpuState, width, 828),
    DEFINE_PROP_UINT32("height", ApplePvgpuState, height, 1792),
};

static void apple_pvgpu_class_init(ObjectClass* klass, const void* data)
{
    ResettableClass* rc = RESETTABLE_CLASS(klass);
    DeviceClass*     dc = DEVICE_CLASS(klass);
    (void)data;

    rc->phases.hold = apple_pvgpu_reset_hold;
    dc->desc        = "Apple Paravirtualized Metal GPU (metal2vulkan)";
    dc->realize     = apple_pvgpu_realize;
    device_class_set_props(dc, apple_pvgpu_props);
    set_bit(DEVICE_CATEGORY_DISPLAY, dc->categories);
}

OBJECT_DEFINE_SIMPLE_TYPE_INSTANCE_INIT(ApplePvgpuState, apple_pvgpu, APPLE_PVGPU, SYS_BUS_DEVICE)
