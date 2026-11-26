#include "InfernoWireClient.hpp"

#include <algorithm>
#include <array>
#include <cstring>
#include <thread>

namespace inferno::gpu {

namespace {

inline uint32_t align8(uint32_t n) { return (n + 7u) & ~7u; }

inline uint32_t atomic_load_u32(const uint8_t* ptr) {
    return reinterpret_cast<const std::atomic<uint32_t>*>(ptr)->load(std::memory_order_acquire);
}

inline void atomic_store_u32(uint8_t* ptr, uint32_t v) {
    reinterpret_cast<std::atomic<uint32_t>*>(ptr)->store(v, std::memory_order_release);
}

inline bool is_power_of_two(uint32_t x) { return x != 0 && (x & (x - 1)) == 0; }

} // namespace

bool WireRing::init(uint8_t* region_base,
                    size_t region_size,
                    size_t header_off,
                    size_t data_off,
                    uint32_t capacity_bytes) {
    if (!region_base || (header_off % 64) != 0 || !is_power_of_two(capacity_bytes) || capacity_bytes < 64) {
        return false;
    }
    if (header_off + sizeof(IvgRingHeader) > region_size || data_off + capacity_bytes > region_size) {
        return false;
    }
    IvgRingHeader hdr{};
    hdr.producer      = 0;
    hdr.consumer      = 0;
    hdr.capacity_mask = capacity_bytes - 1;
    hdr.flags         = IVG_RING_FLAG_HOST_IDLE;
    hdr.data_offset   = static_cast<uint64_t>(data_off);
    std::memcpy(region_base + header_off, &hdr, sizeof(hdr));
    return attach(region_base, region_size, header_off);
}

bool WireRing::attach(uint8_t* region_base, size_t region_size, size_t header_off) {
    if (!region_base || (header_off % 64) != 0 || header_off + sizeof(IvgRingHeader) > region_size) {
        return false;
    }
    region_base_ = region_base;
    region_size_ = region_size;
    header_off_  = header_off;
    size_t   dummy_off = 0;
    uint32_t dummy_cap = 0, dummy_mask = 0;
    return read_geometry(dummy_off, dummy_cap, dummy_mask);
}

bool WireRing::read_geometry(size_t& out_data_off, uint32_t& out_cap, uint32_t& out_mask) const {
    if (!region_base_) return false;
    const auto* hdr = reinterpret_cast<const volatile IvgRingHeader*>(region_base_ + header_off_);
    uint32_t mask   = hdr->capacity_mask;
    uint64_t off    = hdr->data_offset;
    uint32_t cap    = mask + 1u;
    if (!is_power_of_two(cap) || cap < 64 || off + cap > region_size_) {
        return false;
    }
    out_data_off = static_cast<size_t>(off);
    out_cap      = cap;
    out_mask     = mask;
    return true;
}

bool WireRing::host_idle() const {
    if (!region_base_) return true;
    return (atomic_load_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, flags)) & IVG_RING_FLAG_HOST_IDLE)
        != 0;
}

void WireRing::set_host_idle(bool idle) {
    if (!region_base_) return;
    auto* fl = reinterpret_cast<std::atomic<uint32_t>*>(region_base_ + header_off_ + offsetof(IvgRingHeader, flags));
    if (idle) {
        fl->fetch_or(IVG_RING_FLAG_HOST_IDLE, std::memory_order_release);
    } else {
        fl->fetch_and(~IVG_RING_FLAG_HOST_IDLE, std::memory_order_release);
    }
}

void WireRing::write_bytes(size_t data_off, uint32_t cap, uint32_t mask, uint32_t cursor, const uint8_t* src, size_t len) {
    size_t start = static_cast<size_t>(cursor & mask);
    size_t first = std::min(len, static_cast<size_t>(cap) - start);
    volatile uint8_t* dst = region_base_ + data_off + start;
    for (size_t i = 0; i < first; ++i) dst[i] = src[i];
    if (first < len) {
        volatile uint8_t* dst2 = region_base_ + data_off;
        for (size_t i = 0; i < len - first; ++i) dst2[i] = src[first + i];
    }
}

void WireRing::read_bytes(size_t data_off, uint32_t cap, uint32_t mask, uint32_t cursor, uint8_t* dst, size_t len) const {
    size_t start = static_cast<size_t>(cursor & mask);
    size_t first = std::min(len, static_cast<size_t>(cap) - start);
    const volatile uint8_t* src = region_base_ + data_off + start;
    for (size_t i = 0; i < first; ++i) dst[i] = src[i];
    if (first < len) {
        const volatile uint8_t* src2 = region_base_ + data_off;
        for (size_t i = 0; i < len - first; ++i) dst[first + i] = src2[i];
    }
}

bool WireRing::push(const void* data, size_t len) {
    size_t   data_off = 0;
    uint32_t cap = 0, mask = 0;
    if (!read_geometry(data_off, cap, mask)) return false;
    uint32_t len32   = static_cast<uint32_t>(len);
    uint32_t aligned = align8(len32);
    if (aligned > cap) return false;

    uint32_t cons = atomic_load_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, consumer));
    uint32_t prod = atomic_load_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, producer));
    uint32_t used = prod - cons;
    if (used > cap || cap - used < aligned) return false;

    write_bytes(data_off, cap, mask, prod, static_cast<const uint8_t*>(data), len);
    if (aligned > len32) {
        uint8_t pad[8]{};
        write_bytes(data_off, cap, mask, prod + len32, pad, aligned - len32);
    }
    atomic_store_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, producer), prod + aligned);
    return true;
}

bool WireRing::pop(std::vector<uint8_t>& out) {
    size_t   data_off = 0;
    uint32_t cap = 0, mask = 0;
    if (!read_geometry(data_off, cap, mask)) return false;

    uint32_t prod  = atomic_load_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, producer));
    uint32_t cons  = atomic_load_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, consumer));
    uint32_t avail = prod - cons;
    if (avail == 0) return false;
    if (avail > cap || avail < sizeof(IvgCompletionRecord)) return false;

    out.resize(sizeof(IvgCompletionRecord));
    read_bytes(data_off, cap, mask, cons, out.data(), sizeof(IvgCompletionRecord));
    uint32_t aligned = align8(static_cast<uint32_t>(sizeof(IvgCompletionRecord)));
    atomic_store_u32(region_base_ + header_off_ + offsetof(IvgRingHeader, consumer), cons + aligned);
    return true;
}

WireClient::WireClient(uint8_t* region_base, size_t region_size, uint32_t context_id, DoorbellCallback doorbell) :
    region_base_(region_base),
    region_size_(region_size),
    context_id_(context_id),
    doorbell_(std::move(doorbell)) {}

bool WireClient::initialize_control_rings(size_t ctrl_sub_hdr_off,
                                          size_t ctrl_sub_data_off,
                                          size_t ctrl_cmp_hdr_off,
                                          size_t ctrl_cmp_data_off,
                                          uint32_t ring_capacity) {
    return ctrl_sub_.init(region_base_, region_size_, ctrl_sub_hdr_off, ctrl_sub_data_off, ring_capacity)
        && ctrl_cmp_.init(region_base_, region_size_, ctrl_cmp_hdr_off, ctrl_cmp_data_off, ring_capacity);
}

uint64_t WireClient::alloc_handle() {
    return next_handle_.fetch_add(1, std::memory_order_relaxed);
}

size_t WireClient::alloc_staging(size_t bytes, size_t align) {
    size_t aligned = (bytes + align - 1) & ~(align - 1);
    size_t prev    = staging_cursor_.fetch_add(aligned, std::memory_order_relaxed);
    if (prev + aligned > region_size_) {
        staging_cursor_.store(0x200000 + aligned, std::memory_order_relaxed);
        return 0x200000;
    }
    return prev;
}

void WireClient::ring_doorbell(uint64_t queue_id) {
    if (doorbell_) doorbell_(queue_id);
}

bool WireClient::wait_completion(WireRing& cmp_ring, uint64_t seqno, IvgCompletionRecord& out_cmp) {
    std::vector<uint8_t> buf;
    for (int iter = 0; iter < 5000; ++iter) {
        while (cmp_ring.pop(buf)) {
            if (buf.size() >= sizeof(IvgCompletionRecord)) {
                IvgCompletionRecord rec{};
                std::memcpy(&rec, buf.data(), sizeof(rec));
                if (rec.seqno == seqno) {
                    out_cmp = rec;
                    return true;
                }
            }
        }
        std::this_thread::yield();
    }
    return false;
}

bool WireClient::submit_control_pod(uint16_t opcode,
                                    const void* payload,
                                    size_t payload_len,
                                    IvgCompletionRecord& out_cmp) {
    return submit_control_with_tail(opcode, payload, payload_len, nullptr, 0, out_cmp);
}

bool WireClient::submit_control_with_tail(uint16_t opcode,
                                          const void* payload,
                                          size_t payload_len,
                                          const void* tail,
                                          size_t tail_len,
                                          IvgCompletionRecord& out_cmp) {
    uint64_t seq = next_seqno_.fetch_add(1, std::memory_order_relaxed);
    IvgRecordHeader hdr{};
    hdr.opcode     = opcode;
    hdr.size       = static_cast<uint32_t>(sizeof(IvgRecordHeader) + payload_len + tail_len);
    hdr.context_id = context_id_;
    hdr.seqno      = seq;

    std::vector<uint8_t> pkt(hdr.size);
    std::memcpy(pkt.data(), &hdr, sizeof(hdr));
    if (payload && payload_len > 0) {
        std::memcpy(pkt.data() + sizeof(hdr), payload, payload_len);
    }
    if (tail && tail_len > 0) {
        std::memcpy(pkt.data() + sizeof(hdr) + payload_len, tail, tail_len);
    }

    if (!ctrl_sub_.push(pkt.data(), pkt.size())) return false;
    if (ctrl_sub_.host_idle()) ring_doorbell(0);
    return wait_completion(ctrl_cmp_, seq, out_cmp);
}

bool WireClient::ping(uint64_t cookie, IvgCompletionRecord& out_cmp) {
    IvgPingPayload p{cookie};
    return submit_control_pod(IVG_OP_PING, &p, sizeof(p), out_cmp);
}

bool WireClient::get_device_caps(size_t caps_staging_off, IvgDeviceCaps& out_caps) {
    IvgGetCapsPayload   p{static_cast<uint64_t>(caps_staging_off)};
    IvgCompletionRecord cmp{};
    if (!submit_control_pod(IVG_OP_GET_DEVICE_CAPS, &p, sizeof(p), cmp) || cmp.status != 0) {
        return false;
    }
    if (caps_staging_off + sizeof(IvgDeviceCaps) > region_size_) return false;
    std::memcpy(&out_caps, region_base_ + caps_staging_off, sizeof(out_caps));
    return true;
}

bool WireClient::create_context(uint32_t pid) {
    IvgContextCreatePayload p{pid, 0};
    IvgCompletionRecord     cmp{};
    return submit_control_pod(IVG_OP_CONTEXT_CREATE, &p, sizeof(p), cmp) && cmp.status == 0;
}

bool WireClient::create_queue(uint64_t queue_id,
                              size_t sub_hdr_off,
                              size_t sub_data_off,
                              size_t cmp_hdr_off,
                              size_t cmp_data_off,
                              uint32_t capacity,
                              WireRing& out_sub,
                              WireRing& out_cmp) {
    if (!out_sub.init(region_base_, region_size_, sub_hdr_off, sub_data_off, capacity)
        || !out_cmp.init(region_base_, region_size_, cmp_hdr_off, cmp_data_off, capacity))
    {
        return false;
    }
    IvgQueueCreatePayload p{};
    p.queue_id            = queue_id;
    p.sub_ring_offset     = static_cast<uint64_t>(sub_hdr_off);
    p.cmp_ring_offset     = static_cast<uint64_t>(cmp_hdr_off);
    p.max_command_buffers = 16;
    IvgCompletionRecord cmp{};
    return submit_control_pod(IVG_OP_QUEUE_CREATE, &p, sizeof(p), cmp) && cmp.status == 0;
}

void WireClient::sha256(const uint8_t* data, size_t len, uint8_t out_digest[32]) {
    static constexpr uint32_t K[64] = {
        0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
        0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
        0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
        0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
        0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
        0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
        0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
        0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u, 0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
    };
    auto rotr = [](uint32_t x, uint32_t n) { return (x >> n) | (x << (32 - n)); };
    uint32_t h[8] = {
        0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au, 0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u,
    };

    uint64_t bit_len = static_cast<uint64_t>(len) * 8u;
    std::vector<uint8_t> padded(data, data + len);
    padded.push_back(0x80);
    while ((padded.size() % 64) != 56) padded.push_back(0);
    for (int i = 7; i >= 0; --i) padded.push_back(static_cast<uint8_t>((bit_len >> (i * 8)) & 0xFF));

    for (size_t chunk = 0; chunk < padded.size(); chunk += 64) {
        uint32_t w[64]{};
        for (int i = 0; i < 16; ++i) {
            size_t o = chunk + i * 4;
            w[i]     = (uint32_t(padded[o]) << 24) | (uint32_t(padded[o + 1]) << 16)
                     | (uint32_t(padded[o + 2]) << 8) | uint32_t(padded[o + 3]);
        }
        for (int i = 16; i < 64; ++i) {
            uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i]        = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; ++i) {
            uint32_t S1  = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            uint32_t ch  = (e & f) ^ ((~e) & g);
            uint32_t t1  = hh + S1 + ch + K[i] + w[i];
            uint32_t S0  = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
            uint32_t t2  = S0 + maj;
            hh = g; g = f; f = e; e = d + t1;
            d = c; c = b; b = a; a = t1 + t2;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d;
        h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
    for (int i = 0; i < 8; ++i) {
        out_digest[i * 4 + 0] = static_cast<uint8_t>((h[i] >> 24) & 0xFF);
        out_digest[i * 4 + 1] = static_cast<uint8_t>((h[i] >> 16) & 0xFF);
        out_digest[i * 4 + 2] = static_cast<uint8_t>((h[i] >> 8) & 0xFF);
        out_digest[i * 4 + 3] = static_cast<uint8_t>(h[i] & 0xFF);
    }
}

} // namespace inferno::gpu
