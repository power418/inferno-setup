/*
 * Inferno vGPU Firmware / Prelink Package Binary ABI (IVGF).
 *
 * Shared contract between `inferno-vgpu-firmware` (Rust staging tool) and
 * Inferno's simulated iBoot loader (`hw/arm/iboot/inferno_pvgpu_boot.c`).
 */
#ifndef INFERNO_FIRMWARE_ABI_H
#define INFERNO_FIRMWARE_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IVG_FW_MAGIC   0x46475649u /* "IVGF" little-endian */
#define IVG_FW_VERSION 1u

#define IVG_FW_HEADER_SIZE 128u
#define IVG_FW_ENTRY_SIZE  256u
#define IVG_FW_MAX_ENTRIES 16u

/* Package header flags */
#define IVG_FW_FLAG_ENABLE_METAL_FEATURESET (1u << 0)
#define IVG_FW_FLAG_HAS_KEXT                (1u << 1)
#define IVG_FW_FLAG_HAS_BUNDLE              (1u << 2)

/* Entry kinds */
#define IVG_FW_ENTRY_KEXT       1u
#define IVG_FW_ENTRY_BUNDLE     2u
#define IVG_FW_ENTRY_DT_OVERLAY 3u

/* Entry flags */
#define IVG_FW_ENTRY_FLAG_ARM64E   (1u << 0)
#define IVG_FW_ENTRY_FLAG_PRELINK  (1u << 1)
#define IVG_FW_ENTRY_FLAG_REQUIRED (1u << 2)

typedef struct ivg_fw_header {
    uint32_t magic;
    uint32_t version;
    uint32_t header_size;
    uint32_t flags;
    uint64_t total_size;
    uint32_t entry_count;
    uint16_t min_ios_major;
    uint16_t min_ios_minor;
    uint8_t  payload_sha256[32];
    char     featureset_class[32];
    uint8_t  reserved[32];
} ivg_fw_header_t;

typedef struct ivg_fw_entry {
    uint32_t kind;
    uint32_t flags;
    uint64_t payload_offset;
    uint64_t payload_size;
    uint64_t xml_offset;
    uint64_t xml_size;
    uint8_t  payload_sha256[32];
    char     identifier[64];
    char     target_path[120];
} ivg_fw_entry_t;

#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(ivg_fw_header_t) == IVG_FW_HEADER_SIZE, "ivg_fw_header_t must be 128 bytes");
_Static_assert(sizeof(ivg_fw_entry_t) == IVG_FW_ENTRY_SIZE, "ivg_fw_entry_t must be 256 bytes");
#endif

#ifdef __cplusplus
}
#endif

#endif /* INFERNO_FIRMWARE_ABI_H */
