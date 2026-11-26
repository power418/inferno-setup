/*
 * Inferno vGPU iBoot Firmware / Kernelcache Injection Infrastructure.
 *
 * Loads and verifies an `InfernoVgpu.fw` (IVGF) guest package at iBoot time,
 * annotates the `arm-io/inferno-vgpu` Device Tree node, and prepares kernelcache
 * prelink placement for guest `arm64e` kext payloads.
 */
#ifndef HW_ARM_IBOOT_INFERNO_PVGPU_BOOT_H
#define HW_ARM_IBOOT_INFERNO_PVGPU_BOOT_H

#include "hw/arm/dt.h"
#include "hw/arm/iboot/boot.h"
#include "hw/arm/iboot/inferno_firmware_abi.h"
#include <stdbool.h>
#include <stdint.h>

typedef struct InfernoPvgpuFirmware {
    uint8_t*        raw_bytes;
    size_t          raw_size;
    ivg_fw_header_t header;
    ivg_fw_entry_t  entries[IVG_FW_MAX_ENTRIES];
    uint32_t        entry_count;
    bool            valid;
} InfernoPvgpuFirmware;

/* Loads and SHA-256 verifies an IVGF package from `path`. Returns true on success. */
bool inferno_pvgpu_fw_load(const char* path, InfernoPvgpuFirmware* out_fw);

/* Releases buffers owned by `fw`. */
void inferno_pvgpu_fw_free(InfernoPvgpuFirmware* fw);

/* Annotates `root` Device Tree (`arm-io/inferno-vgpu` and optionally `product/graphics-featureset-class`). */
void inferno_pvgpu_fw_apply_dt(const InfernoPvgpuFirmware* fw, AppleDTNode* root);

/* Validates kernelcache layout and logs 16 KiB-aligned kext injection plan for any KEXT entries. */
void inferno_pvgpu_fw_prepare_kernelcache(const InfernoPvgpuFirmware* fw, MachoHeader64* kc);

#endif /* HW_ARM_IBOOT_INFERNO_PVGPU_BOOT_H */
