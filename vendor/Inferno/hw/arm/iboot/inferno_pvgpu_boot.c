/*
 * Inferno vGPU iBoot Firmware / Kernelcache Injection Infrastructure.
 */

#include "qemu/osdep.h"
#include "hw/arm/iboot/inferno_pvgpu_boot.h"
#include "qemu/error-report.h"
#include <glib.h>

static bool verify_sha256_bytes(const uint8_t* data, size_t len, const uint8_t expected[32])
{
    GChecksum* ck = g_checksum_new(G_CHECKSUM_SHA256);
    if (ck == NULL) { return false; }
    g_checksum_update(ck, data, (gssize)len);
    uint8_t digest[32] = {0};
    gsize   dlen       = sizeof(digest);
    g_checksum_get_digest(ck, digest, &dlen);
    g_checksum_free(ck);
    return dlen == sizeof(digest) && memcmp(digest, expected, sizeof(digest)) == 0;
}

void inferno_pvgpu_fw_free(InfernoPvgpuFirmware* fw)
{
    if (fw == NULL) { return; }
    g_free(fw->raw_bytes);
    memset(fw, 0, sizeof(*fw));
}

bool inferno_pvgpu_fw_load(const char* path, InfernoPvgpuFirmware* out_fw)
{
    gchar*  contents = NULL;
    gsize   length   = 0;
    GError* gerr     = NULL;

    memset(out_fw, 0, sizeof(*out_fw));
    if (path == NULL || path[0] == '\0') { return false; }

    if (!g_file_get_contents(path, &contents, &length, &gerr)) {
        warn_report("apple-pvgpu: failed to read firmware '%s': %s", path,
                    gerr != NULL ? gerr->message : "unknown error");
        g_clear_error(&gerr);
        return false;
    }

    if (length < sizeof(ivg_fw_header_t)) {
        warn_report("apple-pvgpu: firmware '%s' too small (%zu bytes)", path, (size_t)length);
        g_free(contents);
        return false;
    }

    ivg_fw_header_t hdr;
    memcpy(&hdr, contents, sizeof(hdr));
    if (hdr.magic != IVG_FW_MAGIC || hdr.version != IVG_FW_VERSION || hdr.header_size != IVG_FW_HEADER_SIZE) {
        warn_report("apple-pvgpu: firmware '%s' bad magic/version (0x%08x v%u)", path, hdr.magic, hdr.version);
        g_free(contents);
        return false;
    }
    if (hdr.total_size != (uint64_t)length || hdr.entry_count > IVG_FW_MAX_ENTRIES) {
        warn_report("apple-pvgpu: firmware '%s' invalid size/entries (%" PRIu64 " vs %zu, entries=%u)",
                    path, hdr.total_size, (size_t)length, hdr.entry_count);
        g_free(contents);
        return false;
    }

    if (!verify_sha256_bytes((const uint8_t*)contents + sizeof(ivg_fw_header_t),
                             (size_t)length - sizeof(ivg_fw_header_t), hdr.payload_sha256))
    {
        warn_report("apple-pvgpu: firmware '%s' SHA-256 body checksum mismatch", path);
        g_free(contents);
        return false;
    }

    size_t table_end = sizeof(ivg_fw_header_t) + (size_t)hdr.entry_count * sizeof(ivg_fw_entry_t);
    if (table_end > (size_t)length) {
        warn_report("apple-pvgpu: firmware '%s' truncated entry table", path);
        g_free(contents);
        return false;
    }

    for (uint32_t i = 0; i < hdr.entry_count; i++) {
        ivg_fw_entry_t entry;
        memcpy(&entry, contents + sizeof(ivg_fw_header_t) + (size_t)i * sizeof(ivg_fw_entry_t), sizeof(entry));
        if (entry.payload_offset > (uint64_t)length || entry.payload_size > (uint64_t)length - entry.payload_offset) {
            warn_report("apple-pvgpu: firmware '%s' entry %u payload out of bounds", path, i);
            g_free(contents);
            return false;
        }
        if (!verify_sha256_bytes((const uint8_t*)contents + entry.payload_offset,
                                 (size_t)entry.payload_size, entry.payload_sha256))
        {
            warn_report("apple-pvgpu: firmware '%s' entry %u SHA-256 mismatch", path, i);
            g_free(contents);
            return false;
        }
        if (entry.xml_size > 0
            && (entry.xml_offset > (uint64_t)length || entry.xml_size > (uint64_t)length - entry.xml_offset))
        {
            warn_report("apple-pvgpu: firmware '%s' entry %u XML out of bounds", path, i);
            g_free(contents);
            return false;
        }
        out_fw->entries[i] = entry;
    }

    out_fw->raw_bytes   = (uint8_t*)contents;
    out_fw->raw_size    = (size_t)length;
    out_fw->header      = hdr;
    out_fw->entry_count = hdr.entry_count;
    out_fw->valid       = true;

    info_report("apple-pvgpu: loaded IVGF v%u firmware '%s' (%zu bytes, %u entries, featureset='%.31s')",
                hdr.version, path, (size_t)length, hdr.entry_count, hdr.featureset_class);
    return true;
}

void inferno_pvgpu_fw_apply_dt(const InfernoPvgpuFirmware* fw, AppleDTNode* root)
{
    if (fw == NULL || !fw->valid || root == NULL) { return; }

    AppleDTNode* vgpu = apple_dt_get_node(root, "arm-io/inferno-vgpu");
    if (vgpu != NULL) {
        apple_dt_set_prop_u32(vgpu, "fw-loaded", 1);
        apple_dt_set_prop_u32(vgpu, "fw-version", fw->header.version);
        apple_dt_set_prop_u32(vgpu, "fw-entries", fw->entry_count);
        apple_dt_set_prop_u32(vgpu, "fw-flags", fw->header.flags);
        if (fw->header.featureset_class[0] != '\0') {
            char fs[33] = {0};
            memcpy(fs, fw->header.featureset_class, 32);
            apple_dt_set_prop_str(vgpu, "fw-featureset", fs);
        }
    }

    /*
     * Only promote `product/graphics-featureset-class` when the package explicitly
     * opts in AND carries both a verified KEXT and Metal BUNDLE payload, so
     * overlay-only or incomplete packages never break software fallback boot.
     */
    const uint32_t full_mask = IVG_FW_FLAG_ENABLE_METAL_FEATURESET | IVG_FW_FLAG_HAS_KEXT | IVG_FW_FLAG_HAS_BUNDLE;
    if ((fw->header.flags & full_mask) == full_mask && fw->header.featureset_class[0] != '\0') {
        AppleDTNode* product = apple_dt_get_node(root, "product");
        if (product != NULL) {
            char fs[33] = {0};
            memcpy(fs, fw->header.featureset_class, 32);
            apple_dt_set_prop_str(product, "graphics-featureset-class", fs);
            info_report("apple-pvgpu: DT product/graphics-featureset-class set to '%s'", fs);
        }
    }
}

void inferno_pvgpu_fw_prepare_kernelcache(const InfernoPvgpuFirmware* fw, MachoHeader64* kc)
{
    if (fw == NULL || !fw->valid || kc == NULL) { return; }

    vaddr kc_base = 0;
    vaddr kc_end  = 0;
    apple_boot_get_kc_bounds(kc, NULL, &kc_base, &kc_end, NULL, NULL);

    uint64_t next_va = (uint64_t)ROUND_UP(kc_end, 0x4000ULL);
    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind != IVG_FW_ENTRY_KEXT) { continue; }
        char id[65] = {0};
        memcpy(id, e->identifier, 64);
        uint64_t mapped = ROUND_UP(e->payload_size, 0x4000ULL);
        if (mapped == 0) { mapped = 0x4000ULL; }
        info_report("apple-pvgpu: planned kernelcache prelink for kext '%s' (kc_type=%u, va=0x%" PRIx64
                    ", size=0x%" PRIx64 ", xml=%" PRIu64 "B)",
                    id, kc->file_type, next_va, mapped, e->xml_size);
        next_va += mapped;
    }
}
