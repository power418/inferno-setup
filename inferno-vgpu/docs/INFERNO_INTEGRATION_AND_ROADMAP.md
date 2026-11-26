# Integrasi `inferno-setup`, Roadmap, & Rencana Pengujian

Dokumen ini menjelaskan cara menghubungkan repositori terpisah `inferno-vgpu` ke `inferno-setup` (`vendor/Inferno`, `config.env`, `setup.sh`, `run.sh`), urutan fase implementasi, rencana pengujian, serta format Accuracy Ledger.

---

## 1. Integrasi ke `inferno-setup`

### 1.1 Submodule & Layout di `inferno-setup`
Tambahkan `inferno-vgpu` sebagai submodule di samping `Inferno` dan `metal2vulkan`:
```ini
[submodule "vendor/Inferno"]
    path = vendor/Inferno
    url = https://git.chefkiss.dev/AppleHax/Inferno.git
    branch = master

[submodule "vendor/metal2vulkan"]
    path = vendor/metal2vulkan
    url = https://github.com/power418/metal2vulkan.git
    branch = master

[submodule "vendor/inferno-vgpu"]
    path = vendor/inferno-vgpu
    url = <url-repo-inferno-vgpu-kamu>
    branch = main
```

### 1.2 Integrasi Build di `vendor/Inferno/xmake.lua`
Inferno menggunakan **XMake** (bukan Meson). Saat opsi `--inferno_vgpu=y` aktif:
1. Build `inferno-vgpu` terlebih dahulu via `cargo xtask dist` → menghasilkan `target/dist-vulkan/{include/inferno_vgpu.h, lib/libinferno_vgpu.a, lib/pkgconfig/inferno-vgpu.pc}`.
2. Di `vendor/Inferno/xmake/options.lua`, tambah opsi `inferno_vgpu` (default `false`).
3. Di `vendor/Inferno/hw/display/xmake.lua`, kompilasi `apple_pvgpu.c` saat `has_config("inferno_vgpu")`.
4. Di `vendor/Inferno/xmake.lua`, tambahkan include dir, `libinferno_vgpu.a`, serta flag `Libs.private` (seperti `-lvulkan`, `-ldl`, `-lpthread`, `-lm`, dan library SDL3 jika window aktif).

### 1.3 Integrasi `config.env`, `setup.sh`, dan `run.sh`

**`config.env`:**
```bash
# off | pv-direct | pv-daemon
GPU="off"
GPU_SERVER_SOCKET="/run/user/$UID/inferno-gpu.sock"
GPU_LOG="${TMPDIR:-/tmp}/inferno-vgpu.log"
GPU_WINDOW="0"   # 0 = readback ke displaypipe/VNC, 1 = SDL3 Vulkan window langsung
```

**`setup.sh`:**
- `./setup.sh gpu`:
  1. Menjalankan `cargo xtask dist` di `vendor/inferno-vgpu`.
  2. (Jika toolchain iOS tersedia) build `guest/bundle` & `guest/driver`, atau menyalin prebuilt.
  3. Rebuild `vendor/Inferno` dengan `--inferno_vgpu=y`.
  4. Menjalankan `fspatcher` untuk memasang `libInfernoMetal.dylib` ke `InfernoData/root`.

**`run.sh`:**
- `./run.sh gpu`: menyalakan `inferno-vgpu-server` di background (jika `GPU="pv-daemon"`).
- `./run.sh main`: jika `GPU != "off"`, tambahkan parameter device `-device apple-pvgpu,...` ke perintah Inferno.
- Fallback otomatis ke `GPU="off"` jika server gagal start atau tidak ada Vulkan GPU yang kompatibel.

---

## 2. Roadmap Implementasi (Phase 0 – Phase 6)

| Fase | Fokus Utama | Deliverable | Exit Criteria |
|---|---|---|---|
| **Phase 0: Feasibility** | Workspace Rust + transport guest minimal | Repo `inferno-vgpu` terbentuk, `metal2vulkan` terhubung, `apple_pvgpu.c` skeleton di `t8030.c`, prototipe doorbell guest | 1 perintah `OP_PING` dari userspace iOS 14 sampai ke host Rust dan dibalas balik ke guest |
| **Phase 1: Core & Wire** | `inferno-vgpu-graphics` + `inferno-vgpu-c` + Shared Region | `inferno_graphics_wire.h` via cbindgen, Ring 0 + Queue Ring, alokasi blok memori (`memfd`/`udmabuf`) | Guest mengalokasikan `MTLBuffer`, mengisi data, dan host memverifikasi byte yang sama tanpa corruption |
| **Phase 2: Compute** | `RtPlatform` (`metal2vulkan-runtime`) + `libInfernoMetal.dylib` compute | `MTLDevice`, `MTLBuffer`, `MTLLibrary` (AIR → SPIR-V), `MTLComputePipelineState`, `MTLComputeCommandEncoder` | Kernel compute Metal (mis. vector add / matrix mul) di guest menghasilkan output akurat dari GPU host |
| **Phase 3: Render Offscreen** | Render pipeline penuh di `RtPlatform` & shim | `MTLTexture`, `MTLRenderPassDescriptor`, `MTLRenderPipelineState`, vertex/fragment shaders, blit readback | Segitiga & tekstur di-render di GPU host dan pixel readback di guest 100% cocok dengan referensi |
| **Phase 4: Present (Readback)** | Integrasi `IOSurface` + `apple_displaypipe_v4` | `OP_SURFACE_IMPORT`, `OP_DISPLAY_PRESENT`, copy hasil Vulkan ke framebuffer Inferno | Aplikasi uji Metal di guest tampil live di layar emulator (`vncview.py`) |
| **Phase 5: CoreAnimation & SpringBoard** | Private SPI Metal iOS 14 + multi-context | Logging SPI di shim, `MTLSharedEvent`, multi-proses (`SpringBoard`, `backboardd`, app) | Home screen & animasi SpringBoard iOS 14 dikomposisi oleh GPU host dengan FPS lebih tinggi dari baseline TCG |
| **Phase 6: Zero-Copy & Stabilitas** | `inferno-vgpu-window` (SDL3 Vulkan) / `dma-buf`, pipeline cache disk, crash recovery | Presentasi tanpa readback CPU, `Device::quiesce`, stress test & fuzzing decoder | Uji 1 jam tanpa crash; mematikan GPU server menghasilkan device lost tanpa SEP panic |

---

## 3. Rencana Pengujian (`TEST_PLAN`)

1. **Unit Test & Fake Platform (`cargo xtask check`):**
   - Menguji seluruh logika `core`, ring buffer, ordering, dan decoder wire menggunakan `core::testkit::FakePlatform` tanpa butuh GPU fisik.
   - Verifikasi `cbindgen` header tidak drift (`cargo xtask header --check`) dan `testkit` tidak bocor ke build rilis (`cargo xtask testkit`).
2. **Host Vulkan Conformance:**
   - Menjalankan fixture shader (`tests/shaders/*.metallib`) langsung di `RtPlatform` (`metal2vulkan-runtime`) pada GPU host (RADV/NVIDIA/ANV) dengan Vulkan Validation Layers aktif.
3. **Fuzzing Decoder Wire:**
   - Semua input dari guest dianggap *untrusted*; jalankan fuzzing pada decoder `inferno-vgpu-graphics` untuk memastikan payload korup tidak memicu panic/OOB di host.
4. **Guest End-to-End Test App (`tests/guest-app/`):**
   - Binary uji iOS 14 yang otomatis menjalankan: (a) ping-pong, (b) compute buffer, (c) render triangle + texture sampling, (d) `IOSurface` present.
5. **Resilience & SEP Safety Test:**
   - Uji `kill -9` pada `inferno-vgpu-server` di tengah frame render → pastikan Inferno melaporkan device lost dan **SEP tidak panic**.

---

## 4. Template Accuracy Ledger (`ACCURACY_LEDGER.md`)

Ikuti disiplin `reims-vgpu/docs/accuracy-ledger.md` dengan mencatat setiap perbedaan antara Metal iOS 14 (Apple A13) dan hasil `metal2vulkan-runtime` di Vulkan host:

| Fitur Metal (iOS 14 / A13) | Status di `metal2vulkan` | Perilaku di `inferno-vgpu` | Catatan / Mitigasi |
|---|---|---|---|
| Compute & Render dasar (AIR → SPIR-V) | Didukung | Eksekusi native di Vulkan | — |
| `MTLStorageModeMemoryless` (TBDR tile memory) | Emulasi / fallback | Dialokasikan sebagai attachment transient Vulkan | Evaluasi performa di Phase 5 |
| Tile Shaders / Imageblocks (Apple Family 5+) | Parsial / Refusal | Kembalikan refusal terstruktur jika belum didukung | Pantau log `QuartzCore` iOS 14 |
| `MTLHeap` placement & aliasing | Bergantung driver | Ikuti kapabilitas `metal2vulkan-runtime` | Catat di `DeviceCaps` |
| Private SPI `QuartzCore` (`MTLTexture` IOSurface) | Custom shim | Diimplementasikan bertahap dari log Phase 5 | Tambah test per-SPI |
