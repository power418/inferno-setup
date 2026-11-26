/*
 * Inferno vGPU iBoot Firmware / Kernelcache Injection Infrastructure.
 *
 * Loads and verifies an `InfernoVgpu.fw` (IVGF) guest package at iBoot time,
 * annotates the `arm-io/inferno-vgpu` Device Tree node, merges CDHash entries
 * into XNU Static TrustCache, and prepares/stages kernelcache prelink kexts.
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
    bool            trustcache_merged;
    uint32_t        trustcache_added;
    uint32_t        reloc_count;
    bool            kext_injected;
} InfernoPvgpuFirmware;

/* Loads and SHA-256 verifies an IVGF package from `path`. Returns true on success. */
bool inferno_pvgpu_fw_load(const char* path, InfernoPvgpuFirmware* out_fw);

/* Releases buffers owned by `fw`. */
void inferno_pvgpu_fw_free(InfernoPvgpuFirmware* fw);

/* Returns extra virtual/buffer headroom (in bytes) required above `kc_end` for kext injection. */
uint64_t inferno_pvgpu_fw_kc_headroom(const InfernoPvgpuFirmware* fw);

/*
 * Merges all `IVG_FW_ENTRY_CDHASH` entries into an XNU TrustCache v1/v2 buffer
 * (allocated as `ROUND_UP_16K(8 + header + count * entry_size)` with `{1, 8}` wrapper).
 * Entries are sorted lexicographically and deduplicated for XNU binary search.
 */
bool inferno_pvgpu_fw_merge_trustcache(InfernoPvgpuFirmware* fw, uint8_t** tc_buf, uint64_t* tc_size);

/* Annotates `root` Device Tree (`arm-io/inferno-vgpu` and optionally `product/graphics-featureset-class`). */
void inferno_pvgpu_fw_apply_dt(const InfernoPvgpuFirmware* fw, AppleDTNode* root);

/* Validates kernelcache layout, counts reloc map entries, and stages/logs prelink kext injection. */
void inferno_pvgpu_fw_prepare_kernelcache(InfernoPvgpuFirmware* fw, MachoHeader64* kc, uint64_t headroom);

#endif /* HW_ARM_IBOOT_INFERNO_PVGPU_BOOT_H */
