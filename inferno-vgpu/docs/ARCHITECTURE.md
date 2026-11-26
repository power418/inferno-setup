# Arsitektur Teknis — `inferno-vgpu`

Dokumen ini menjabarkan arsitektur `inferno-vgpu` yang mengadopsi pola desain **`reims-vgpu`** (Rust + `metal2vulkan-runtime` + C FFI + wire protocol) namun disesuaikan khusus untuk emulator iPhone **ChefKiss Inferno (`t8030` / iOS 14)**.

---

## 1. Gambaran Umum End-to-End

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                    GUEST: iOS 14.0 (t8030 / arm64e)                     │
│                                                                         │
│  iOS App / SpringBoard / QuartzCore                                     │
│         │  (Metal API calls: MTLDevice, MTLCommandQueue, MTLLibrary...) │
│         ▼                                                               │
│  libInfernoMetal.dylib  (guest/bundle, di-inject lewat fspatcher)       │
│         │  Serialisasi Metal calls -> wire records (inferno_graphics_wire.h)
│         │  Alokasi handle 64-bit, gpuAddress & gpuResourceID            │
│         ▼                                                               │
│  Guest Transport Driver (guest/driver: IOKit IOUserClient / MMIO map)   │
│         │  Tulis ke Shared Region Ring + bunyikan Doorbell MMIO         │
└─────────┼───────────────────────────────────────────────────────────────┘
          │  (MMIO Doorbell + Shared Region + IRQ)
          ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                EMULATOR: ChefKiss Inferno (QEMU / C)                    │
│                                                                         │
│  hw/display/apple_pvgpu.c  (terpasang di t8030.c: SysBus MMIO / apcie)  │
│         │                                                               │
│         ├── Mode A (Direct Static Link): panggil C API `ivg_*`          │
│         └── Mode B (SEP-Safe Daemon): forward via Unix socket + memfd   │
└─────────┼───────────────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                  HOST ENGINE: inferno-vgpu (RUST)                       │
│                                                                         │
│  inferno-vgpu-c (FFI `ivg_*`)  /  inferno-vgpu-server (IPC daemon)      │
│         │                                                               │
│         ▼                                                               │
│  inferno-vgpu  ◄─────── inferno-vgpu-graphics (rings, control, ops)     │
│         │                                                               │
│         ▼                                                               │
│  inferno-vgpu-vulkan  (core + RtPlatform)                               │
│         │                                                               │
│         ├── metal2vulkan          (AIR -> SPIR-V, pipeline cache, types)│
│         ├── metal2vulkan-runtime  (Metal-shaped API -> Vulkan 1.3+)     │
│         └── inferno-vgpu-window   (SDL3 + Vulkan swapchain / readback)  │
│                                                                         │
│         ▼                                                               │
│  Host Vulkan Driver (RADV / NVIDIA / ANV)  ──►  GPU Fisik Host          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Crate Graph & Tanggung Jawab (Mirip `reims-vgpu`)

```text
metal2vulkan, metal2vulkan-runtime
          ▲
  inferno-vgpu-vulkan     (core + RtPlatform + OS Linux services + shaders)
          ▲
  inferno-vgpu-window     (SDL3 + Vulkan swapchain, opsional untuk direct present)
          ▲
     inferno-vgpu   ◄──── inferno-vgpu-graphics (wire format, rings, device state)
          ▲
    ┌─────┴──────────────────┐
    ▼                        ▼
inferno-vgpu-c      inferno-vgpu-server
(staticlib/cdylib    (proses terpisah,
 untuk XMake)         Unix socket + memfd)
```

### Detail Tiap Crate

1. **`crates/inferno-vgpu-vulkan`** *(padanan `reims-vgpu-vulkan`)*:
   - **`core` module**: mendefinisikan trait `Platform` (generic over `P: Platform`), `Handle` (tanpa `Clone`, pakai `.clone_ref()`, `Drop` = release), command enums (`BlitCmd`, `ComputeCmd`, `RenderCmd`), descriptor values, `MemoryMap`, `DisplayPipelineCache`, dan `testkit` (fake platform untuk unit test tanpa GPU).
   - **`rt_platform` module**: implementasi `RtPlatform` di atas `metal2vulkan-runtime` dan tipe enum Metal dari `metal2vulkan`. Satu instance device per proses (`singleton`).
   - **`os/linux` module**: alokasi `RegionMemory` lewat `memfd_create`, impor `udmabuf` (AMD/Intel) atau pointer import (NVIDIA), `eventfd`, dan `exit_gate` untuk teardown aman.
   - **`metallibs/`**: shader presentasi internal (blit `BGRA8`/NV12 → swapchain) dalam bentuk `.metallib` precompiled sehingga build Linux tidak butuh Xcode.

2. **`crates/inferno-vgpu-graphics`** *(padanan `reims-vgpu-graphics`)*:
   - Tidak bergantung pada Vulkan/platform — murni protokol wire, layout register MMIO, ring submission/completion, control ring, dan opcode encoder.
   - Menghasilkan `include/inferno_graphics_wire.h` lewat `cbindgen` agar **host Rust**, **Inferno C**, dan **guest iOS C++/Obj-C++** memakai definisi struct byte-identical.

3. **`crates/inferno-vgpu-window`** *(padanan `reims-vgpu-window`)*:
   - Mengelola window SDL3 + Vulkan swapchain langsung dari `VkImage` hasil `metal2vulkan-runtime` di thread UI tersendiri (`inferno-vgpu.ui`), untuk presentasi zero-copy di fase akhir.

4. **`crates/inferno-vgpu`** *(padanan `reims-vgpu`)*:
   - Menghubungkan decoder wire `inferno-vgpu-graphics` dengan `RtPlatform` (`inferno-vgpu-vulkan`), manajemen resource per-context (tiap proses iOS = 1 context), dan presentasi display.

5. **`crates/inferno-vgpu-c`** *(padanan `reims-vgpu-c`)*:
   - Lapisan FFI C tipis (`ivg_graphics_device_create`, `ivg_graphics_device_mmio_write`, `ivg_graphics_device_doorbell_write`, `ivg_graphics_device_read_display`, `ivg_quiesce_for_exit`, dll.).
   - Semua fungsi `extern "C"` dengan panic hook abort-safe agar panic Rust tidak pernah unwind ke kode C Inferno.
   - Output `cargo xtask dist`: `libinferno_vgpu.a`, `libinferno_vgpu.so`, `inferno_vgpu.h`, dan `inferno-vgpu.pc`.

6. **`crates/inferno-vgpu-server`** *(tambahan khusus Inferno untuk proteksi SEP)*:
   - Binary Rust terpisah yang membungkus `inferno-vgpu` di balik Unix domain socket (`/run/user/$UID/inferno-gpu.sock`) + passing FD `memfd` (`SCM_RIGHTS`).
   - Memungkinkan Inferno tetap hidup jika driver Vulkan host crash, sehingga mencegah **SEP panic** akibat shutdown mendadak.

---

## 3. Keputusan Desain Utama (Adaptasi dari `reims-vgpu`)

| # | Keputusan | Alasan |
|---|---|---|
| **D-01** | **Potongan di level Metal API, bukan AGX hardware** | Meniru firmware/command stream GPU Apple A13 (AGX) memerlukan RE masif. Memotong di level Metal API membuat `metal2vulkan` langsung bekerja dan shader AIR diterjemahkan di CPU host (bukan di TCG guest yang lambat). |
| **D-02** | **Satu ring & satu host worker per `MTLCommandQueue`** | Menghindari bottleneck global lock; encoding di guest (encoder N+1) berjalan paralel saat host menurunkan encoder N ke Vulkan. |
| **D-03** | **Guest memilih `gpuAddress` & `gpuResourceID`** | Host melakukan *pinning* alamat tersebut di address space `metal2vulkan`. Pembuatan buffer tidak butuh round-trip sinkron ke host, dan argument buffer tidak perlu di-patch. |
| **D-04** | **Tidak ada Rust reference ke guest RAM** | Guest vCPU menulis secara konkuren. Akses memori guest hanya lewat raw copy (`HostRange`) atau impor halaman ke Vulkan (`udmabuf` / host pointer). |
| **D-05** | **Memory model mengikuti GPU host (`hasUnifiedMemory`)** | Pada dGPU host: Private buffer & texture di VRAM, Shared di host-cached RAM, Managed punya dua salinan (`didModifyRange` / `synchronizeResource`). Pada iGPU: satu salinan. |
| **D-06** | **iOS Unified Memory Shim** | Di iPhone asli (A13), semua memori bersifat unified (`MTLStorageModeShared` sangat dominan). Guest shim iOS memetakan `Shared` ke blok pool host-cached yang dapat dibaca GPU host tanpa copy per-draw. |
| **D-07** | **Refusal terstruktur, bukan crash** | Fitur Metal yang tidak didukung `metal2vulkan` dicatat dan dikembalikan sebagai error Metal yang sah ke guest; proses hanya mati pada kondisi fatal yang memang fatal di Metal. |
| **D-08** | **Dua mode koneksi Inferno (Direct C API vs Daemon)** | Fase 0–3 dapat memakai static link `inferno-vgpu-c` (debug cepat) atau langsung daemon `inferno-vgpu-server` untuk isolasi penuh terhadap SEP. |

---

## 4. Model Concurrency & Threading

- **Control Thread (1 per device):** memproses Control Ring (ring 0) — create/destroy context, alokasi blok memori/pool, registrasi `IOSurface`, dan perubahan mode display.
- **Queue Worker Threads (1 per active `MTLCommandQueue`):** polling lockless submission ring; saat ring kosong, thread tidur di `eventfd` dan dibangunkan hanya saat guest menulis Doorbell MMIO.
- **Display / Present Thread:** mengelola `DisplayPipelineCache` dan ekspor frame ke `apple_displaypipe_v4` (readback) atau SDL3 Vulkan swapchain (`inferno-vgpu-window`).
- **Exit Gate & Quiesce:** saat Inferno keluar, `ivg_quiesce_for_exit()` menghentikan worker dan memanggil `Device::quiesce()` pada `metal2vulkan-runtime` sebelum proses berakhir agar destructor driver Vulkan (khususnya NVIDIA) tidak `SIGSEGV`.

---

## 5. Sinkronisasi `Cargo.toml` dengan `vendor/metal2vulkan`

Di `inferno-vgpu/Cargo.toml`, arahkan dependency ke `metal2vulkan` yang sama (baik via git rev maupun local path untuk pengembangan bersama `inferno-setup`):

```toml
[workspace]
resolver = "3"
members = [
    "crates/inferno-vgpu-vulkan",
    "crates/inferno-vgpu-window",
    "crates/inferno-vgpu-graphics",
    "crates/inferno-vgpu",
    "crates/inferno-vgpu-c",
    "crates/inferno-vgpu-server",
    "xtask",
]

[workspace.dependencies]
metal2vulkan = { path = "../inferno-setup/vendor/metal2vulkan/crates/metal2vulkan" }
metal2vulkan-runtime = { path = "../inferno-setup/vendor/metal2vulkan/crates/metal2vulkan-runtime" }
ash = { version = "0.38", default-features = false, features = ["std", "loaded"] }
parking_lot = "0.12.5"
zerocopy = { version = "0.8.27", features = ["derive"] }
log = { version = "0.4.34", features = ["kv"] }
```

*(Saat dipublikasikan sebagai repo terpisah, ganti `path` dengan `git = "https://github.com/power418/metal2vulkan", rev = "<pinned-commit>"`.)*
