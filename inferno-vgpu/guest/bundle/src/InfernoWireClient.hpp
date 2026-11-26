#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <string>
#include <vector>

#include "inferno_graphics_wire.h"

namespace inferno::gpu {

using DoorbellCallback = std::function<void(uint64_t queue_id)>;

/// Portable lockless SPSC ring wrapper operating on shared memory inside Region 1.
class WireRing {
public:
    WireRing() = default;

    bool init(uint8_t* region_base, size_t region_size, size_t header_off, size_t data_off, uint32_t capacity_bytes);
    bool attach(uint8_t* region_base, size_t region_size, size_t header_off);

    bool push(const void* data, size_t len);
    bool pop(std::vector<uint8_t>& out);

    bool host_idle() const;
    void set_host_idle(bool idle);

private:
    bool read_geometry(size_t& out_data_off, uint32_t& out_cap, uint32_t& out_mask) const;
    void write_bytes(size_t data_off, uint32_t cap, uint32_t mask, uint32_t cursor, const uint8_t* src, size_t len);
    void read_bytes(size_t data_off, uint32_t cap, uint32_t mask, uint32_t cursor, uint8_t* dst, size_t len) const;

    uint8_t* region_base_ = nullptr;
    size_t   region_size_ = 0;
    size_t   header_off_  = 0;
};

/// Guest wire client serializing Metal operations onto `apple-pvgpu` rings.
class WireClient {
public:
    WireClient(uint8_t* region_base, size_t region_size, uint32_t context_id, DoorbellCallback doorbell);

    bool initialize_control_rings(size_t ctrl_sub_hdr_off,
                                  size_t ctrl_sub_data_off,
                                  size_t ctrl_cmp_hdr_off,
                                  size_t ctrl_cmp_data_off,
                                  uint32_t ring_capacity);

    bool ping(uint64_t cookie, IvgCompletionRecord& out_cmp);
    bool get_device_caps(size_t caps_staging_off, IvgDeviceCaps& out_caps);
    bool create_context(uint32_t pid);

    bool surface_import(uint32_t surface_id,
                        uint64_t texture_id,
                        uint32_t width,
                        uint32_t height,
                        uint32_t pixel_format,
                        uint64_t bytes_per_row,
                        uint64_t region_offset = UINT64_MAX);
    bool surface_release(uint64_t texture_id);
    bool display_present(uint64_t texture_id, uint32_t level = 0, uint32_t slice = 0);

    bool has_unified_memory() const { return caps_loaded_ && cached_caps_.has_unified_memory != 0; }
    const IvgDeviceCaps& cached_caps() const { return cached_caps_; }

    uint64_t alloc_handle();
    size_t   alloc_staging(size_t bytes, size_t align = 64);

    bool submit_control_pod(uint16_t opcode, const void* payload, size_t payload_len, IvgCompletionRecord& out_cmp);
    bool submit_control_with_tail(uint16_t opcode,
                                  const void* payload,
                                  size_t payload_len,
                                  const void* tail,
                                  size_t tail_len,
                                  IvgCompletionRecord& out_cmp);

    bool create_queue(uint64_t queue_id,
                      size_t sub_hdr_off,
                      size_t sub_data_off,
                      size_t cmp_hdr_off,
                      size_t cmp_data_off,
                      uint32_t capacity,
                      WireRing& out_sub,
                      WireRing& out_cmp);

    uint32_t context_id() const { return context_id_; }
    uint8_t* region_base() const { return region_base_; }
    size_t   region_size() const { return region_size_; }
    void     ring_doorbell(uint64_t queue_id);

    static void sha256(const uint8_t* data, size_t len, uint8_t out_digest[32]);

private:
    bool wait_completion(WireRing& cmp_ring, uint64_t seqno, IvgCompletionRecord& out_cmp);

    uint8_t*              region_base_;
    size_t                region_size_;
    uint32_t              context_id_;
    DoorbellCallback      doorbell_;
    WireRing              ctrl_sub_;
    WireRing              ctrl_cmp_;
    std::atomic<uint64_t> next_seqno_{1};
    std::atomic<uint64_t> next_handle_{1};
    std::atomic<size_t>   staging_cursor_{0x200000}; /* 2 MiB past control/queue ring area */
    IvgDeviceCaps         cached_caps_{};
    bool                  caps_loaded_{false};
};

} // namespace inferno::gpu
