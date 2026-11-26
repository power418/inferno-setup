#ifndef INFERNO_VGPU_H
#define INFERNO_VGPU_H

#pragma once

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint32_t IvgStatus;

#define IVG_OK                    0u
#define IVG_ERR_INVALID_ARGUMENT  1u
#define IVG_ERR_UNSUPPORTED       2u
#define IVG_ERR_HOST_RESOURCES    3u
#define IVG_ERR_GPU               4u
#define IVG_ERR_INTERNAL          5u

typedef struct IvgGraphicsDevice IvgGraphicsDevice;

typedef void (*IvgRaiseIrqFn)(void* ctx, uint32_t irq_bits);
typedef void (*IvgDestroyFn)(void* ctx);

typedef struct IvgGraphicsCallbacks {
    uint32_t      struct_size;
    void*         ctx;
    IvgRaiseIrqFn raise_irq;
    IvgDestroyFn  destroy;
} IvgGraphicsCallbacks;

typedef struct IvgGraphicsDeviceConfig {
    uint32_t struct_size;
    uint64_t region_size;
    uint32_t display_width;
    uint32_t display_height;
    uint32_t use_fake_platform;
} IvgGraphicsDeviceConfig;

const char* ivg_status_message(IvgStatus status);

IvgStatus ivg_graphics_device_create(
    const IvgGraphicsDeviceConfig* config,
    const IvgGraphicsCallbacks* callbacks,
    IvgGraphicsDevice** out_device);

void ivg_graphics_device_release(IvgGraphicsDevice* device);

void* ivg_graphics_device_region_ptr(const IvgGraphicsDevice* device);
int ivg_graphics_device_region_fd(const IvgGraphicsDevice* device);
size_t ivg_graphics_device_region_size(const IvgGraphicsDevice* device);

uint32_t ivg_graphics_device_mmio_read(IvgGraphicsDevice* device, uint64_t offset);
void ivg_graphics_device_mmio_write(IvgGraphicsDevice* device, uint64_t offset, uint32_t value);
void ivg_graphics_device_doorbell_write(IvgGraphicsDevice* device, uint64_t queue_id);
void ivg_graphics_device_reset(IvgGraphicsDevice* device);

IvgStatus ivg_graphics_device_read_display(
    IvgGraphicsDevice* device,
    uint8_t* out_bgra8,
    size_t out_len,
    uint32_t* out_width,
    uint32_t* out_height);

bool ivg_quiesce_for_exit(void);

#ifdef __cplusplus
}
#endif

#endif /* INFERNO_VGPU_H */
