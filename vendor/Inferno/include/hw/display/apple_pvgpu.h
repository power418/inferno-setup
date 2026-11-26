/*
 * Apple Paravirtualized Metal GPU (reims/metal2vulkan bridge) for Inferno.
 */
#pragma once

#include "qemu/osdep.h"
#include "qom/object.h"

#define TYPE_APPLE_PVGPU "apple-pvgpu"
OBJECT_DECLARE_SIMPLE_TYPE(ApplePvgpuState, APPLE_PVGPU);
