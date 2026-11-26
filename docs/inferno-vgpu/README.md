# inferno-vgpu — Blueprint & Dokumentasi Proyek

**Target:** Virtual GPU berakselerasi penuh untuk emulator iPhone (**ChefKiss Inferno**, `t8030` / iPhone 11, iOS 14) menggunakan **Rust** dan **`metal2vulkan` / `metal2vulkan-runtime`** di atas Vulkan host.  
**Referensi Arsitektur:** Adaptasi struktur dan desain [`reims-vgpu`](https://github.com/power418/reims-vgpu) (yang dibangun untuk macOS VM) ke ekosistem **iOS / iPhone emulator (Inferno)**.

---

## 1. Tujuan Proyek Terpisah

Proyek `inferno-vgpu` dirancang sebagai repositori mandiri (di luar `vendor/Inferno` dan `inferno-setup`) agar:

1. **Struktur dan codebase Rust tetap bersih** — identik secara arsitektural dengan `reims-vgpu`, memakai Cargo workspace + `metal2vulkan` + `xtask`.
2. **Mudah dihubungkan ke `inferno-setup`** — menghasilkan artefak standar (`libinferno_vgpu.a` / `.so`, `inferno_vgpu.h`, `inferno-vgpu.pc`, atau binary daemon `inferno-vgpu-server`) yang dikonsumsi `vendor/Inferno` (XMake) dan `setup.sh` / `run.sh`.
3. **Pemisahan batas lisensi & stabilitas** — crash di driver Vulkan host tidak boleh merusak proses QEMU/Inferno dan state SEP iOS.

---

## 2. Struktur Direktori Proyek (`inferno-vgpu`)

Struktur berikut meniru `reims-vgpu` secara langsung, disesuaikan untuk target iPhone/iOS (menghilangkan komponen khusus macOS seperti OpenCore, UEFI Option ROM, dan Ambrosia, serta menggantinya dengan target iOS 14 / `t8030` / `fspatcher`):

```text
inferno-vgpu/
├── Cargo.toml                        # Rust workspace (edition 2024) + pin/path metal2vulkan
├── Cargo.lock
├── rust-toolchain.toml               # Pin nightly toolchain + rustfmt + clippy
├── Makefile                          # Orchestration build: dist, guest, inferno-link, check
├── .cargo/config.toml                # Alias `cargo xtask` & flags compiler
│
├── crates/                           # === HOST SIDE (RUST) ===
│   ├── inferno-vgpu-vulkan/          # Core + `RtPlatform` di atas `metal2vulkan` & `metal2vulkan-runtime`
│   │   ├── Cargo.toml
│   │   ├── build.rs                  # Verifikasi & embed precompiled present shaders (.metallib)
│   │   ├── metallibs/                # Precompiled shaders untuk presentasi (kompatibel build Linux)
│   │   └── src/
│   │       ├── core/                 # Platform trait, MemoryMap, DisplayPipelineCache, Raise/Sync
│   │       ├── rt_platform/          # Implementasi Metal-shaped API -> metal2vulkan-runtime -> Vulkan
│   │       ├── device_choice.rs      # Pemilihan GPU Vulkan host (RADV / NVIDIA / ANV)
│   │       └── os/linux/             # memfd, mmap, udmabuf, timeline/eventfd
│   │
│   ├── inferno-vgpu-graphics/        # Wire protocol device, ring buffer, control channel, ops
│   │   ├── Cargo.toml
│   │   ├── cbindgen.toml
│   │   ├── include/
│   │   │   └── inferno_graphics_wire.h   # Header C shared antara host Rust, Inferno C, dan guest iOS
│   │   └── src/
│   │       ├── wire.rs               # Definisi struct/opcode wire (zerocopy)
│   │       ├── ring.rs               # Lockless submission & completion rings
│   │       ├── control.rs            # Control ring (context, memory block, display, fence)
│   │       ├── ops/                  # Decoder blit, compute, render, library/pipeline
│   │       └── device.rs             # State machine device & worker per-queue
│   │
│   ├── inferno-vgpu-window/          # Window native opsional (SDL3 + Vulkan swapchain langsung)
│   │   └── src/
│   │
│   ├── inferno-vgpu/                 # Public Rust API: menghubungkan wire graphics -> RtPlatform -> window
│   │   └── src/
│   │
│   ├── inferno-vgpu-c/               # Thin C FFI (`ivg_*`) untuk link statis/dinamis ke Inferno (C)
│   │   ├── cbindgen.toml
│   │   ├── include/
│   │   │   └── inferno_vgpu.h
│   │   └── src/
│   │
│   └── inferno-vgpu-server/          # (Opsional / Rekomendasi SEP-safe) Daemon proses terpisah via Unix socket + memfd
│       └── src/
│
├── guest/                            # === GUEST SIDE (iOS 14 arm64 / arm64e) ===
│   ├── Makefile                      # Cross-compile shim & driver untuk arm64-apple-ios14.0
│   ├── bundle/                       # `libInfernoMetal.dylib` / `InfernoMetal.bundle` (Obj-C++ / C++)
│   │   ├── Info.plist
│   │   └── src/                      # Implementasi MTLDevice, MTLCommandQueue, MTLBuffer, MTLTexture,
│   │                                 # MTLLibrary (AIR), MTLRenderCommandEncoder, MTLComputeCommandEncoder,
│   │                                 # CAMetalLayer / IOSurface interop
│   ├── driver/                       # Transport kernel iOS (IOKit IOUserClient / MMIO bridge t8030)
│   │   └── src/                      # Mapping Shared Region, doorbell write, IRQ completion wake
│   ├── fspatch/                      # Manifest & script injeksi ke dyld_shared_cache via Inferno `fspatcher`
│   └── tools/                        # Checker simbol iOS kernelcache / dyld_shared_cache
│
├── inferno-device/                   # === PATCH / MODULE UNTUK `vendor/Inferno` (C) ===
│   ├── apple_pvgpu.c                 # Device QEMU/Inferno (MMIO SysBus atau PCIe di `apcie`)
│   ├── apple_pvgpu.h
│   └── xmake_integration.lua         # Snippet integrasi XMake untuk link `libinferno_vgpu.a`
│
├── xtask/                            # Build automation (`cargo xtask dist|header|check|testkit`)
├── tests/                            # Unit test, shader `.metal`/`.metallib` fixtures, & iOS guest test app
│   ├── shaders/
│   └── guest-app/                    # App uji iOS minimal (compute add, triangle, texture, IOSurface)
├── scripts/                          # Script setup toolchain, build, dan sync ke `inferno-setup`
└── docs/                             # Dokumentasi proyek ini
```

---

## 3. Peta Dokumen

| Dokumen | Isi & Tujuan |
|---|---|
| [REQUIREMENTS.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/inferno-vgpu/REQUIREMENTS.md) | Kebutuhan fungsional, non-fungsional, hardware/OS host, Vulkan floor, Rust toolchain, XMake, dan toolchain cross-compile iOS 14. |
| [ARCHITECTURE.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/inferno-vgpu/ARCHITECTURE.md) | Arsitektur end-to-end, crate graph Rust + `metal2vulkan`, keputusan desain (adaptasi dari `reims-vgpu`), memory model, concurrency, dan isolasi crash SEP. |
| [PROTOCOL_AND_TRANSPORT.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/inferno-vgpu/PROTOCOL_AND_TRANSPORT.md) | Spesifikasi wire protocol (`inferno_graphics_wire.h`), layout MMIO/BAR `t8030`, ring buffer, handle 64-bit, address pinning, dan jalur AIR → SPIR-V. |
| [IOS_GUEST_INTEGRATION.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/inferno-vgpu/IOS_GUEST_INTEGRATION.md) | Desain sisi guest iOS 14: `libInfernoMetal.dylib`, injeksi via `fspatcher`, opsi transport kernel (Phase 0), `QuartzCore`/`SpringBoard` SPI, dan `IOSurface`/display. |
| [INFERNO_INTEGRATION_AND_ROADMAP.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/inferno-vgpu/INFERNO_INTEGRATION_AND_ROADMAP.md) | Integrasi ke `inferno-setup` (`xmake.lua`, `t8030.c`, `config.env`, `run.sh`, `setup.sh`), roadmap Phase 0–6, test plan, dan accuracy ledger. |

---

## 4. Hubungan dengan Dokumen Lama

Dokumen ini memperluas dan mengoperasionalkan [PRD_GPU_ACCELERATION.md](file:///home/ahmadzanisy/Desktop/inferno-setup/docs/PRD_GPU_ACCELERATION.md) dengan mengadopsi struktur crate Rust dan pipeline `metal2vulkan` yang sudah terbukti berjalan di `reims-vgpu`.
