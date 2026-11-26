# `inferno-vgpu/firmware/`

Boot-time firmware package (`InfernoVgpu.fw` / `IVGF`) and simulated-iBoot loader infrastructure for ChefKiss Inferno (`t8030`, iOS 14+), mirroring `reims-vgpu`'s `firmware/` + `firmware/ambrosia/` staging architecture.

## Contents

- `include/inferno_firmware_abi.h`: C11 binary contract (`ivg_fw_header_t` 128 B, `ivg_fw_entry_t` 256 B) shared between Rust `inferno-vgpu-firmware` and Inferno iBoot.
- `iboot/inferno_pvgpu_boot.{h,c}`: Inferno simulated-iBoot loader (`-M t8030,pvgpu-fw=<path>`) that verifies SHA-256 checksums, annotates `arm-io/inferno-vgpu`, conditionally promotes `product/graphics-featureset-class`, and plans 16 KiB-aligned kernelcache prelink placement.
- `dt-overlay.plist` & `set.toml`: Package manifest describing Device Tree overlay, guest kext, and Metal bundle entries.
- `Makefile`: Stages and verifies `build/InfernoVgpu.fw` via `inferno-vgpu-fw`.

## Usage

```bash
# 1. Build and verify an overlay-only IVGF package on Linux (safe fallback):
make -C firmware all

# 2. Inspect a real iOS 14 kernelcache (raw Mach-O or IM4P/LZFSE):
make -C firmware kc-inspect KC=/path/to/kernelcache.release.iphone12b

# 3. Stage a full Metal package once iOS arm64e kext + bundle are compiled with Xcode/iPhoneOS SDK:
../target/release/inferno-vgpu-fw stage \
  --out build/InfernoVgpu.fw \
  --featureset MTL2,2 --enable-metal \
  --kext dev.inferno.driver.InfernoVgpuDriver:../guest/build/InfernoVgpuDriver.kext/InfernoVgpuDriver:../guest/driver/Info.plist \
  --bundle dev.inferno.metal.InfernoMetal:../guest/build/InfernoMetal.bundle/InfernoMetal:/System/Library/Extensions/AGXMetalA13.bundle/AGXMetalA13
```
