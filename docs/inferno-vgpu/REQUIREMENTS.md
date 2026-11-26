# Requirements — `inferno-vgpu` (iPhone / Inferno Hardware Acceleration)

Dokumen ini mendefinisikan seluruh persyaratan teknis, sistem, toolchain, dan kriteria kualitas untuk membangun `inferno-vgpu` sebagai proyek terpisah yang mengakselerasi grafis Metal di emulator iPhone (ChefKiss Inferno, `t8030` / iPhone 11, iOS 14) menggunakan **Rust** dan **`metal2vulkan`**.

---

## 1. Functional Requirements (FR)

| ID | Kebutuhan | Detail & Kriteria Penerimaan |
|---|---|---|
| **FR-01** | **Metal Compute Acceleration** | Aplikasi iOS 14 yang membuat `MTLDevice`, `MTLCommandQueue`, `MTLBuffer`, `MTLLibrary` (AIR/metallib), dan `MTLComputePipelineState` mengeksekusi kernel di GPU host lewat `metal2vulkan-runtime` dan mengembalikan output yang akurat. |
| **FR-02** | **Metal Render Acceleration** | Render pass offscreen & onscreen (`MTLTexture`, `MTLRenderPassDescriptor`, `MTLRenderPipelineState`, vertex/fragment shaders, depth/stencil, blending, sampler) berjalan di Vulkan host. |
| **FR-03** | **AIR Shader Translation** | Bytecode Apple IR (`.metallib` / AIR) dikirim dari guest iOS ke host satu kali (dideduplikasi via SHA-256), diterjemahkan oleh `metal2vulkan` menjadi SPIR-V, dan di-cache di disk/memori host. |
| **FR-04** | **CoreAnimation / SpringBoard Compositing** | `QuartzCore` / `SpringBoard` pada iOS 14 mengenali device Metal paravirtual, membuat `IOSurface`-backed `MTLTexture`, dan melakukan komposisi UI lewat GPU host. |
| **FR-05** | **Display & Presentasi** | Mendukung 2 mode presentasi: (1) **Compatibility/Readback** ke framebuffer `apple_displaypipe_v4` (tampil di VNC / `vncview.py`), dan (2) **Zero-Copy Direct Present** lewat `reims-vgpu-window` (SDL3 + Vulkan swapchain) atau `dma-buf` scanout. |
| **FR-06** | **Structured Refusal & Diagnostics** | Fitur Metal yang belum didukung `metal2vulkan` menghasilkan refusal terstruktur (`AccuracyLedger` + log host + error Metal valid di guest), bukan corruption diam-diam. |
| **FR-07** | **Toggleable Fallback** | Dapat dimatikan penuh melalui `GPU="off"` di `config.env`, mengembalikan Inferno ke jalur framebuffer CPU `apple_displaypipe_v4` tanpa mengubah boot flow. |
| **FR-08** | **Header & ABI Generation** | Header wire (`inferno_graphics_wire.h`) dan header C API (`inferno_vgpu.h`) di-generate otomatis oleh `cbindgen` dan diverifikasi oleh `cargo xtask header --check`. |

---

## 2. Non-Functional Requirements (NFR)

| ID | Kategori | Target |
|---|---|---|
| **NFR-01** | **SEP & Emulator Crash Isolation** | Crash/panic pada driver Vulkan host atau shader compiler **tidak boleh** mematikan proses Inferno secara paksa saat menjalankan SEP, atau minimal harus ditangani via *device lost* & *graceful quiesce* (mengikuti mekanisme `exit_gate` + `Device::quiesce` di `reims-vgpu` atau mode daemon terpisah `inferno-vgpu-server`). |
| **NFR-02** | **Memory Safety & Undefined Behavior** | Crate `core` melarang `unsafe_code` di root; tidak boleh ada Rust reference (`&[u8]`) langsung ke memori guest RAM yang bisa diubah vCPU secara konkuren (akses hanya via `HostRange` / copy / Vulkan import). |
| **NFR-03** | **Zero-Copy Path** | Buffer besar (`Shared`/`Managed`) dan `IOSurface` diimpor ke Vulkan via `memfd` + `/dev/udmabuf` (AMD/Intel) atau `VK_EXT_external_memory_host` (NVIDIA) tanpa menyalin data per-frame lewat CPU guest. |
| **NFR-04** | **Latency & Concurrency** | Setiap `MTLCommandQueue` memiliki submission/completion ring sendiri dan worker thread sendiri di host (tidak ada global lock tunggal untuk decode semua queue). |
| **NFR-05** | **Linux-First Host Build** | Build host Rust (`inferno-vgpu-*`) **harus bisa dilakukan 100% di Linux tanpa macOS/Xcode** (shader internal presentasi disimpan sebagai precompiled `.metallib` terverifikasi SHA-256 seperti pola `reims-vgpu-vulkan/metallibs/`). |
| **NFR-06** | **Minimal Upstream Diff** | Perubahan pada `vendor/Inferno` dibatasi seminimal mungkin (perangkat baru di file terpisah + hook minimal di `t8030.c` dan `xmake.lua`) agar tidak konflik saat upstream Inferno di-update. |

---

## 3. Host System & Hardware Requirements (Linux x86_64)

### 3.1 OS & Kernel
- **OS:** Linux x86_64 (Ubuntu/Debian, Arch/CachyOS, Fedora).
- **Kernel:**
  - **Minimum untuk Inferno TCG:** Linux 6.x dengan dukungan `memfd_create` dan `eventfd`.
  - **Direkomendasikan untuk Zero-Copy / `udmabuf`:** Linux **≥ 6.13** (wajib jika nanti dipakai bersama KVM memory-pool mapping seperti `reims-vgpu`) dengan akses baca/tulis ke `/dev/udmabuf` (user tergabung di grup `kvm` atau aturan `udev`).
- **Tuning NVIDIA (opsional):** Naikkan `vm.max_map_count` jika menggunakan GPU NVIDIA agar pemetaan blok memori besar tidak fallback ke copy.

### 3.2 GPU & Vulkan Driver
Host harus memiliki driver Vulkan yang memenuhi persyaratan floor `metal2vulkan-runtime`:
- **AMD:** Mesa **RADV** (modern Vulkan 1.3+, mendukung `dma-buf` external memory).
- **NVIDIA:** Driver NVIDIA proprietary (Vulkan 1.3+, mendukung `VK_EXT_external_memory_host`).
- **Intel:** Mesa **ANV** (Vulkan 1.3+, mendukung `dma-buf`).
- Paket sistem: `vulkan-loader` (`libvulkan.so.1`), `vulkan-tools` (`vulkaninfo`), dan `vulkan-validationlayers` (untuk pengujian).

---

## 4. Build & Development Toolchain Requirements

### 4.1 Host (Rust + C/C++ + Inferno)
| Komponen | Versi / Spesifikasi | Fungsi |
|---|---|---|
| **Rust (`rustup`)** | Nightly sesuai `rust-toolchain.toml` (seperti `reims-vgpu`: `nightly-2026-11-03`, `edition = 2024`, komponen `rustfmt`, `clippy`) | Kompilasi workspace `inferno-vgpu` dan `metal2vulkan` |
| **`cbindgen`** | `0.29.4` (`cargo install --locked cbindgen --version 0.29.4`) | Generate `inferno_vgpu.h` & `inferno_graphics_wire.h` |
| **`cargo-nextest`** | Terbaru (`cargo install --locked cargo-nextest`) | Menjalankan unit & integration test |
| **`xmake`** | `≥ 3.0.0` | Build `vendor/Inferno` |
| **C/C++ Build Tools** | `clang`, `lld`, `cmake`, `make`, `pkg-config` | Build Inferno, `sdl3-sys`, dan `tikv-jemallocator` |
| **Dependency Lokal** | `vendor/metal2vulkan` (sinkron dengan commit yang didukung) | Translator AIR → SPIR-V dan `metal2vulkan-runtime` |

### 4.2 Guest Side (iOS 14.0 `18A5351d`, `arm64` / `arm64e`)
Untuk membangun `guest/bundle` (`libInfernoMetal.dylib`) dan `guest/driver`:
- **Opsi 1 (Direkomendasikan untuk build stabil):** Apple Silicon / Intel Mac dengan **Xcode** + **iOS SDK** + Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain` jika perlu mengompilasi ulang `.metallib`).
- **Opsi 2 (Cross-compile di Linux):** `clang` + `ld64.lld` (LLVM) + iOS 14 SDK headers/TBD stubs (misalnya via `theos` / `cctools-port` / `osxcross` target `arm64-apple-ios14.0`).
- **Injeksi RootFS:** Tool `fspatcher` bawaan [vendor/Inferno/fspatcher/](file:///home/ahmadzanisy/Desktop/inferno-setup/vendor/Inferno/fspatcher) + IPSW / root image iOS 14.0 (`18A5351d`) yang sudah terpasang di `InfernoData/`.
- **Signing:** Ad-hoc / fake codesign (`ldid` atau `codesign`) yang kompatibel dengan patch AMFI / `SimulatedSigningServer` pada `inferno-setup`.

---

## 5. Scope & Non-Goals (Versi 1.0)

**Masuk Scope v1.0:**
- Metal API (Compute, Render, Blit, Library/AIR, Heap, Buffer, Texture, Fence/Event, `IOSurface`).
- iOS 14.0 (`18A5351d`) pada machine `t8030` (iPhone 11) di ChefKiss Inferno.
- Host Linux x86_64 dengan Vulkan (RADV / NVIDIA / ANV).

**Di Luar Scope v1.0 (Non-Goals):**
- Emulasi register/firmware hardware Apple AGX asli (ASC/RTKit).
- OpenGL ES native (bisa menyusul lewat ANGLE-on-Metal atau lowering seperti `RgfxGLLowering` di `reims-vgpu`).
- Hardware video decode/encode (`VideoToolbox` → VA-API/NVDEC — dipisah ke fase lanjutan sebagai crate `inferno-vgpu-video`).
- Multi-display / HDR / VR.
