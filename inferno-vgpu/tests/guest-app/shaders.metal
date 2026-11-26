#include <metal_stdlib>
using namespace metal;

kernel void vec_add(device const uint32_t* a [[buffer(0)]],
                    device const uint32_t* b [[buffer(1)]],
                    device uint32_t* out     [[buffer(2)]],
                    uint gid                 [[thread_position_in_grid]]) {
    out[gid] = a[gid] + b[gid];
}

struct VSOut {
    float4 position [[position]];
    float4 color;
};

vertex VSOut tri_vert(uint vid [[vertex_id]]) {
    const float2 pos[3] = { float2(0.0, 0.7), float2(-0.7, -0.7), float2(0.7, -0.7) };
    const float4 col[3] = {
        float4(1.0, 0.0, 0.0, 1.0),
        float4(0.0, 1.0, 0.0, 1.0),
        float4(0.0, 0.0, 1.0, 1.0),
    };
    VSOut out;
    out.position = float4(pos[vid], 0.0, 1.0);
    out.color    = col[vid];
    return out;
}

fragment float4 tri_frag(VSOut in [[stage_in]]) {
    return in.color;
}
