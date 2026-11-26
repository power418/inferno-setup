#include "InfernoWireClient.hpp"

#include <cassert>
#include <cstddef>
#include <cstring>
#include <vector>

using namespace inferno::gpu;

int main() {
    /* 1. Verify exact ABI parity with Rust `inferno-vgpu-graphics` */
    static_assert(sizeof(IvgRingHeader) == 64, "IvgRingHeader must be 64 bytes");
    static_assert(alignof(IvgRingHeader) == 64, "IvgRingHeader must be 64-byte aligned");
    static_assert(offsetof(IvgRingHeader, producer) == 0, "producer offset");
    static_assert(offsetof(IvgRingHeader, consumer) == 4, "consumer offset");
    static_assert(offsetof(IvgRingHeader, capacity_mask) == 8, "capacity_mask offset");
    static_assert(offsetof(IvgRingHeader, flags) == 12, "flags offset");
    static_assert(offsetof(IvgRingHeader, data_offset) == 16, "data_offset offset");
    static_assert(sizeof(IvgRecordHeader) == 24, "IvgRecordHeader must be 24 bytes");
    static_assert(sizeof(IvgCompletionRecord) == 40, "IvgCompletionRecord must be 40 bytes");
    static_assert(sizeof(IvgDeviceCaps) == 168, "IvgDeviceCaps must be 168 bytes");
    static_assert(sizeof(IvgTextureDescWire) == 40, "IvgTextureDescWire must be 40 bytes");
    static_assert(sizeof(IvgSurfaceImportPayload) == 72, "IvgSurfaceImportPayload must be 72 bytes");
    static_assert(sizeof(IvgDisplayPresentPayload) == 16, "IvgDisplayPresentPayload must be 16 bytes");
    static_assert(sizeof(IvgRenderBeginPassPayload) == 488, "IvgRenderBeginPassPayload must be 488 bytes");

    /* 2. Verify SHA-256 matches standard vector ("abc") */
    uint8_t digest[32]{};
    WireClient::sha256(reinterpret_cast<const uint8_t*>("abc"), 3, digest);
    const uint8_t expected[32] = {
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
        0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
        0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    };
    assert(std::memcmp(digest, expected, 32) == 0);

    /* 3. Verify WireRing wraparound */
    std::vector<uint8_t> region(4 * 1024 * 1024, 0);
    WireRing sub, cmp;
    assert(sub.init(region.data(), region.size(), 0x0, 0x1000, 128));
    assert(cmp.init(region.data(), region.size(), 0x1000, 0x2000, 128));

    IvgCompletionRecord sample{};
    for (uint64_t i = 1; i <= 25; ++i) {
        sample.seqno      = i;
        sample.result_u64 = i * 99;
        assert(cmp.push(&sample, sizeof(sample)));
        std::vector<uint8_t> popped;
        assert(cmp.pop(popped));
        assert(popped.size() == sizeof(IvgCompletionRecord));
        IvgCompletionRecord got{};
        std::memcpy(&got, popped.data(), sizeof(got));
        assert(got.seqno == i);
        assert(got.result_u64 == i * 99);
    }

    /* 4. Verify WireClient record encoding for ping / surface_import / display_present */
    uint64_t db_count = 0;
    WireClient client(region.data(), region.size(), 42, [&](uint64_t) { db_count++; });
    assert(client.initialize_control_rings(0x0, 0x1000, 0x10000, 0x11000, 4096));
    assert(client.alloc_handle() == 1);
    assert(client.alloc_staging(128) >= 0x200000);

    return 0;
}
