/*
 * Apple Paravirtualized Metal GPU (reims/metal2vulkan bridge) for Inferno.
 */
#pragma once

#include "qemu/osdep.h"
#include "qom/object.h"

#define TYPE_APPLE_PVGPU "apple-pvgpu"
OBJECT_DECLARE_SIMPLE_TYPE(ApplePvgpuState, APPLE_PVGPU);

typedef struct ApplePvgpuCaps {
    uint32_t vendor_id;
    uint32_t device_id;
    uint32_t has_unified_memory;
    uint32_t max_threadgroup_memory_length;
    uint64_t max_buffer_length;
    uint32_t read_write_texture_tier;
    uint32_t argument_buffers_tier;
    uint32_t apple_gpu_family;
    uint32_t supports_function_pointers;
    char     device_name[128];
} ApplePvgpuCaps;

bool     apple_pvgpu_is_connected(const ApplePvgpuState* s);
uint64_t apple_pvgpu_region_size(const ApplePvgpuState* s);
bool     apple_pvgpu_get_caps(const ApplePvgpuState* s, ApplePvgpuCaps* out_caps);
