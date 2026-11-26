/*
 * Inferno vGPU iBoot Firmware / Kernelcache Injection Infrastructure.
 */

#include "qemu/osdep.h"
#include "hw/arm/iboot/inferno_pvgpu_boot.h"
#include "qemu/bswap.h"
#include "qemu/error-report.h"
#include "qemu/units.h"
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

static int compare_cdhash_entry(const void* a, const void* b)
{
    return memcmp(a, b, 20);
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

uint64_t inferno_pvgpu_fw_kc_headroom(const InfernoPvgpuFirmware* fw)
{
    if (fw == NULL || !fw->valid || (fw->header.flags & IVG_FW_FLAG_INJECT_KEXT) == 0) {
        return 0;
    }

    uint64_t total = 0;
    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind != IVG_FW_ENTRY_KEXT) { continue; }
        uint64_t mapped = ROUND_UP(e->payload_size, 0x4000ULL);
        if (mapped == 0) { mapped = 0x4000ULL; }
        total += mapped;
    }
    if (total == 0) { return 0; }
    total += 0x10000ULL;
    if (total > 4ULL * MiB) { total = 4ULL * MiB; }
    return total;
}

bool inferno_pvgpu_fw_merge_trustcache(InfernoPvgpuFirmware* fw, uint8_t** tc_buf, uint64_t* tc_size)
{
    if (fw == NULL || !fw->valid || tc_buf == NULL || *tc_buf == NULL || tc_size == NULL || *tc_size < 32) {
        return false;
    }

    size_t extra_hashes = 0;
    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind != IVG_FW_ENTRY_CDHASH) { continue; }
        if (e->payload_size == 0 || (e->payload_size % 20ULL) != 0) {
            warn_report("apple-pvgpu: ignoring malformed CDHASH entry %u (size=%" PRIu64 ")", i, e->payload_size);
            continue;
        }
        extra_hashes += (size_t)(e->payload_size / 20ULL);
    }
    if (extra_hashes == 0) { return false; }

    const uint8_t* raw = *tc_buf;
    uint32_t       num_tc;
    uint32_t       tc_off;
    memcpy(&num_tc, raw, 4);
    memcpy(&tc_off, raw + 4, 4);
    num_tc = le32_to_cpu(num_tc);
    tc_off = le32_to_cpu(tc_off);
    if (num_tc != 1 || tc_off != 8 || (uint64_t)tc_off + 24 > *tc_size) {
        warn_report("apple-pvgpu: unexpected TrustCache wrapper (count=%u, off=%u)", num_tc, tc_off);
        return false;
    }

    const uint8_t* mod = raw + tc_off;
    uint32_t       version;
    uint32_t       old_count;
    memcpy(&version, mod, 4);
    memcpy(&old_count, mod + 20, 4);
    version   = le32_to_cpu(version);
    old_count = le32_to_cpu(old_count);

    size_t entry_size = (version == 1) ? 22 : ((version == 2) ? 24 : 0);
    if (entry_size == 0) {
        warn_report("apple-pvgpu: unsupported TrustCache version %u", version);
        return false;
    }

    uint64_t mod_bytes = 24ULL + (uint64_t)old_count * entry_size;
    if ((uint64_t)tc_off + mod_bytes > *tc_size) {
        warn_report("apple-pvgpu: TrustCache bounds invalid (entries=%u)", old_count);
        return false;
    }

    size_t   total_cap = (size_t)old_count + extra_hashes;
    uint8_t* scratch   = g_malloc0(total_cap * entry_size);
    memcpy(scratch, mod + 24, (size_t)old_count * entry_size);

    size_t write_idx = old_count;
    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind != IVG_FW_ENTRY_CDHASH || e->payload_size == 0 || (e->payload_size % 20ULL) != 0) {
            continue;
        }
        size_t n = (size_t)(e->payload_size / 20ULL);
        for (size_t j = 0; j < n; j++) {
            uint8_t* dst = scratch + write_idx * entry_size;
            memcpy(dst, fw->raw_bytes + e->payload_offset + j * 20, 20);
            dst[20] = 2; /* CS_HASHTYPE_SHA256 */
            dst[21] = 0; /* flags */
            write_idx++;
        }
    }

    qsort(scratch, write_idx, entry_size, compare_cdhash_entry);

    size_t unique_count = 0;
    for (size_t i = 0; i < write_idx; i++) {
        uint8_t* cur = scratch + i * entry_size;
        if (unique_count > 0 && memcmp(scratch + (unique_count - 1) * entry_size, cur, 20) == 0) {
            continue;
        }
        if (unique_count != i) {
            memcpy(scratch + unique_count * entry_size, cur, entry_size);
        }
        unique_count++;
    }

    uint64_t new_mod_size   = 24ULL + (uint64_t)unique_count * entry_size;
    uint64_t new_alloc_size = ROUND_UP(8ULL + new_mod_size, 0x4000ULL);
    uint8_t* new_buf        = g_malloc0((gsize)new_alloc_size);

    uint32_t w_one     = cpu_to_le32(1);
    uint32_t w_off     = cpu_to_le32(8);
    uint32_t w_ver     = cpu_to_le32(version);
    uint32_t w_new_cnt = cpu_to_le32((uint32_t)unique_count);
    memcpy(new_buf, &w_one, 4);
    memcpy(new_buf + 4, &w_off, 4);
    memcpy(new_buf + 8, &w_ver, 4);
    memcpy(new_buf + 12, mod + 4, 16); /* preserve original UUID */
    memcpy(new_buf + 28, &w_new_cnt, 4);
    memcpy(new_buf + 32, scratch, unique_count * entry_size);

    g_free(scratch);
    g_free(*tc_buf);
    *tc_buf  = new_buf;
    *tc_size = new_alloc_size;

    fw->trustcache_merged = true;
    fw->trustcache_added  = (uint32_t)(unique_count > old_count ? (unique_count - old_count) : extra_hashes);

    info_report("apple-pvgpu: merged %zu CDHash(es) into Static TrustCache (v%u, %u -> %zu entries)",
                extra_hashes, version, old_count, unique_count);
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
        apple_dt_set_prop_u32(vgpu, "fw-trustcache-merged", fw->trustcache_merged ? 1u : 0u);
        apple_dt_set_prop_u32(vgpu, "fw-trustcache-added", fw->trustcache_added);
        apple_dt_set_prop_u32(vgpu, "fw-reloc-entries", fw->reloc_count);
        apple_dt_set_prop_u32(vgpu, "fw-kext-injected", fw->kext_injected ? 1u : 0u);
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

void inferno_pvgpu_fw_prepare_kernelcache(InfernoPvgpuFirmware* fw, MachoHeader64* kc, uint64_t headroom)
{
    if (fw == NULL || !fw->valid || kc == NULL) { return; }

    uint32_t reloc_total = 0;
    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind == IVG_FW_ENTRY_RELOC_MAP) {
            reloc_total += (uint32_t)(e->payload_size / IVG_FW_RELOC_ENTRY_SIZE);
        }
    }
    fw->reloc_count = reloc_total;

    vaddr kc_base = 0;
    vaddr kc_end  = 0;
    apple_boot_get_kc_bounds(kc, NULL, &kc_base, &kc_end, NULL, NULL);
    uint8_t* kc_buf = apple_boot_get_macho_buffer(kc);

    uint64_t next_va    = (uint64_t)ROUND_UP(kc_end, 0x4000ULL);
    uint64_t used_extra = 0;
    bool     do_inject  = (fw->header.flags & IVG_FW_FLAG_INJECT_KEXT) != 0;

    for (uint32_t i = 0; i < fw->entry_count; i++) {
        const ivg_fw_entry_t* e = &fw->entries[i];
        if (e->kind != IVG_FW_ENTRY_KEXT) { continue; }
        char id[65] = {0};
        memcpy(id, e->identifier, 64);

        if (e->payload_size >= 32) {
            uint32_t magic = 0;
            uint32_t cpu   = 0;
            memcpy(&magic, fw->raw_bytes + e->payload_offset, 4);
            memcpy(&cpu, fw->raw_bytes + e->payload_offset + 4, 4);
            if (le32_to_cpu(magic) != MACH_MAGIC_64 || le32_to_cpu(cpu) != CPU_TYPE_ARM64) {
                warn_report("apple-pvgpu: kext '%s' has invalid Mach-O header (0x%08x/0x%08x)", id, magic, cpu);
                continue;
            }
        }

        uint64_t mapped = ROUND_UP(e->payload_size, 0x4000ULL);
        if (mapped == 0) { mapped = 0x4000ULL; }

        if (do_inject && used_extra + mapped <= headroom && kc_buf != NULL && next_va >= kc_base) {
            memcpy(kc_buf + (next_va - kc_base), fw->raw_bytes + e->payload_offset, (size_t)e->payload_size);
            fw->kext_injected = true;
            info_report("apple-pvgpu: staged kext '%s' in KC buffer (kc_type=%u, va=0x%" PRIx64
                        ", size=0x%" PRIx64 ", relocs=%u)",
                        id, kc->file_type, next_va, mapped, reloc_total);
            used_extra += mapped;
        } else {
            info_report("apple-pvgpu: planned kernelcache prelink for kext '%s' (kc_type=%u, va=0x%" PRIx64
                        ", size=0x%" PRIx64 ", xml=%" PRIu64 "B, relocs=%u)",
                        id, kc->file_type, next_va, mapped, e->xml_size, reloc_total);
        }
        next_va += mapped;
    }
}
