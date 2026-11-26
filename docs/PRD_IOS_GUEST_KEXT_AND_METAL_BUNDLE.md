# Inferno iOS Guest Graphics Kext & Metal Bundle (Stage 2/3)

## Product Requirements Document & Technical Design — Part 2

**Project:** `inferno-setup` + `inferno-vgpu` (ChefKiss Inferno, `t8030` / iPhone 11, iOS 14.0 `18A5351d`)
**Predecessor:** [PRD_GPU_ACCELERATION.md](./PRD_GPU_ACCELERATION.md) (Phase 1 — Host `metal2vulkan`, `apple-pvgpu`, IPC & `IVGF` Firmware Infrastructure)
**Reference Architecture:** `power418/reims-vgpu` (`docs/graphics-kext/design.md` — Metal-call stream, `IOGPUFamily`, `MetalPluginName`, `IOSurface` cache sync, dynamic kext binding)
**Host Backends Verified:** NVIDIA GeForce RTX 3050 Laptop GPU (`0x10de:0x25a2`, discrete) & Intel Iris Xe Graphics ADL GT2 (`0x8086`, `unified=true`) via Vulkan + `metal2vulkan`
**Status:** Draft / Ready for Stage 2A Implementation

---

# 1. Ringkasan Eksekutif

[PRD #1](./PRD_GPU_ACCELERATION.md) telah menyelesaikan seluruh pondasi sisi **host**, **perangkat virtual QEMU/Inferno**, **DeviceTree/IORegistry**, dan **infrastruktur paket firmware `IVGF`**. Saat ini emulator iOS 14.0 (`t8030`) sudah berhasil mendeteksi perangkat GPU paravirtual **`arm-io/inferno-vgpu`** secara live di IORegistry kernel XNU lengkap dengan pemetaan fisik `IODeviceMemory`, interrupt AIC, dan identitas GPU host nyata dari `inferno-vgpu-server`.

Namun aplikasi userland iOS (`SpringBoard`, `QuartzCore`, dan app Metal) **belum menerima full Metal acceleration** karena sisi **guest iOS (`arm64e-apple-ios14.0`)** belum memiliki dua biner utama yang terpasang:

1. **Guest Kernel Driver (`InfernoVgpuDriver.kext`)** yang mengikat ke `arm-io/inferno-vgpu` (`apple,inferno-pvgpu`), memetakan ring & doorbell ke userspace, dan mempublikasikan layanan akselerator `IOGPU` / `IOAcceleratorES` di IORegistry.
2. **Guest Metal Plugin Bundle (`InfernoMetal.bundle`)** yang dimuat `Metal.framework` melalui properti `MetalPluginName` / `MetalPluginClassName` untuk menerjemahkan panggilan Metal ke wire stream `inferno-vgpu` (sama seperti arsitektur `reims-vgpu`).

Dokumen ini (PRD #2) mendefinisikan kebutuhan produk, desain teknis rinci, strategi kompilasi & injeksi kernelcache/TrustCache, serta tahapan implementasi untuk mencapai **full Metal acceleration di dalam emulator iPhone iOS 14.0**.

---

# 2. Baseline Terverifikasi & Temuan Riset

## 2.1 Yang Sudah Berjalan (Hasil PRD #1)

| Layer | Komponen | Status Terverifikasi |
|---|---|---|
| Host Translator | `vendor/metal2vulkan` + `inferno-vgpu-vulkan` | **PASS** di **NVIDIA RTX 3050** (`unified=false`) & **Intel Iris Xe** (`unified=true`, Mesa Vulkan 1.4.363) |
| Host GPU Server | `inferno-vgpu/crates/inferno-vgpu-server` | **LIVE** di `/run/user/$UID/inferno-gpu.sock`, mendukung `HELLO`, `GET_CAPS`, MMIO, Doorbell, `memfd` 64 MiB |
| QEMU Device | `vendor/Inferno/hw/display/apple_pvgpu.c` | **LIVE** ter-map di `0x280000000` (Regs 64 KiB), `0x280010000` (Doorbell 64 KiB), `0x290000000` (Shared RAM 64 MiB), IRQ AIC `0x210` |
| DeviceTree & iBoot | `t8030.c`, `iboot.c`, `inferno_pvgpu_boot.c` | **LIVE** — `apple,inferno-pvgpu` lolos whitelist `KEEP_COMP`, `interrupt-parent` terhubung ke `arm-io/aic` (`0x1a`) |
| Live iOS IORegistry | `idevicediagnostics ioregentry inferno-vgpu` | **LIVE (`rc=0`)** — `model`, `vendor-id`, `device-id`, `gpu-backend=metal2vulkan`, `IODeviceMemory[0..2]`, `IOInterruptController0000001A` |
| Firmware Packaging | `inferno-vgpu/crates/inferno-vgpu-firmware` (`inferno-vgpu-fw`) | **PASS** — parser Mach-O (`MH_EXECUTE` & `MH_FILESET`), dekoder `IM4P`/LZFSE, builder & verifier paket `IVGF v1` SHA-256 |
| Guest Source Scaffold | `inferno-vgpu/guest/driver/` & `inferno-vgpu/guest/bundle/` | **HOST ABI PASS** — source C++/Obj-C++ tersedia, `wire_abi_test` lulus di host Linux |

## 2.2 Temuan Kunci dari IPSW iOS 14.0 (`18A5351d`)

Hasil inspeksi langsung terhadap IPSW target `iPhone11,8,iPhone12,1_14.0_18A5351d_Restore.ipsw`:

| Artefak IPSW | Ukuran Decoded | Tipe Mach-O | `LC_SYMTAB` | Catatan Kunci |
|---|---:|---|---|---|
| `kernelcache.release.iphone12b` | 45.071.376 B | `MH_EXECUTE` (`arm64e`) | `nsyms = 0` (stripped) | 247 prelinked bundle IDs (`com.apple.iokit.IOGPUFamily`, `com.apple.AGXG12P`, `com.apple.iokit.IOSurface`, `com.apple.iokit.IOMobileGraphicsFamily`) |
| `kernelcache.research.iphone12b` | 54.613.232 B | `MH_EXECUTE` (`arm64e`) | **`nsyms = 149.571`** (7,1 MB string table) | Berisi **1.403 simbol `IOGPU`**, **76 vtable `IOGPU`/`AGX`**, **895 `IOSurface`**, **7.506 `IOService`**, **301 `IOUserClient`**, **565 `IOMobileFramebuffer`** |

**Implikasi Strategis:** Ketersediaan `kernelcache.research.iphone12b` dengan **149.571 simbol utuh** di dalam IPSW menghilangkan kebutuhan menebak ABI kernel iOS 14.0 secara buta. Seluruh vtable `IOGPU`, `IOGPUDeviceUserClient`, `IOSurface`, dan KPI XNU dapat diekstrak secara deterministik untuk membangun header ABI dan peta binding ke kernelcache `release`.

---

# 3. Pemetaan Desain `reims-vgpu` ke Inferno iOS 14.0

| Aspek Desain | `reims-vgpu` (macOS `arm64e` / Ambrosia) | Adaptasi Inferno iOS 14.0 (`t8030`) | Keuntungan / Perubahan |
|---|---|---|---|
| **Cut Protokol** | Metal-call stream per-queue ring (`submission` + `completion`) ke `metal2vulkan` | **Identik** (`inferno_graphics_wire.h`, shared `memfd` 64 MiB + doorbell) | Sudah terimplementasi & lulus e2e |
| **Bus & Hardware** | `IOPCIDevice` (PCI BAR + MSI-X) | **`AppleARMIODevice`** (`arm-io/inferno-vgpu`, MMIO + AIC wired IRQ `0x210`) | Sudah terbaca live di IORegistry iOS |
| **Kernel Accelerator** | Subclass `IOGPU` / `IOAccelerator` (`IOMatchCategory = IOAccelerator`) | Subclass `IOGPU` / `IOAccelerator` (**`IOMatchCategory = IOAcceleratorES`**) | Mengikuti konvensi `com.apple.AGXG12P` di iOS 14.0 |
| **Metal Discovery** | `/Library/GPUBundles/<name>.bundle` via `MetalPluginName` & `MetalPluginClassName` | **`/System/Library/Extensions/InfernoMetal.bundle`** via `MetalPluginName` & `MetalPluginClassName` | Sesuai path `AGXMetalA13.bundle` di iOS 14.0 |
| **Display Pipeline** | Harus membuat subclass `IOMobileFramebufferLegacy` (445–484 vtable slots) | **Tidak wajib** — `disp0` (`AppleDisplayPipeV4` / `IOMobileFramebuffer`) sudah berjalan | Menghemat kompleksitas besar; cukup sinkronisasi `IOSurface` |
| **Signing & Library Validation** | `RgfxGate.kext` meng-hook 6 MAC policy (`_mac_file_check_library_validation`, dll.) | **Injeksi CDHash ke Static TrustCache di iBoot** (`iboot.c`) | Lebih bersih, stabil, tidak bergantung prototipe MAC internal |
| **Kext Loading & Binding** | OpenCore/Ambrosia (`MH_FILESET`) + `Graft.kext` (symbol-by-name runtime) | **Injeksi `MH_EXECUTE` prelink di `inferno_pvgpu_boot.c`** + symbol/signature table | Mendukung `research` KC (by-symbol) & `release` KC (signature-verified) |
| **Memory Model** | Mengikuti `hasUnifiedMemory` GPU host (discrete two-copy vs UMA) | **Identik** (`unified=false` untuk RTX 3050, `unified=true` untuk Intel Iris Xe) | `GPU_DEVICE=Intel` dapat memanfaatkan mode UMA |

---

# 4. Tujuan dan Non-Goals

## 4.1 Goals

1. **Injeksi Kext Nyata saat Boot:** `inferno_pvgpu_boot.c` menyuntikkan biner Mach-O `InfernoVgpuDriver.kext` dari `InfernoVgpu.fw` ke dalam kernelcache XNU di RAM sehingga `InfernoVgpuService` benar-benar dimuat, memanggil `start()`, membaca `IVG_MAGIC`, dan melakukan ping-pong control ring ke `inferno-vgpu-server`.
2. **TrustCache Code-Signing Tanpa Melemahkan OS:** iBoot mendaftarkan CDHash SHA-256 milik `InfernoMetal.bundle` ke TrustCache sehingga `amfid`/kernel mempercayai bundle di `/System/Library/Extensions/InfernoMetal.bundle`.
3. **Eksposur `IOAcceleratorES` & Discovery Metal:** `MTLCreateSystemDefaultDevice()` di dalam iOS 14.0 mengembalikan `InfernoMTLDevice` (bukan `nil`), dan aplikasi compute/render Metal berjalan di GPU host (`NVIDIA RTX 3050` atau `Intel Iris Xe`).
4. **Integrasi `IOSurface` & Komposisi `QuartzCore` / `SpringBoard`:** `CAMetalLayer` dan `QuartzCore` merender ke `IOSurface` yang diakselerasi lalu tampil ke layar emulator melalui `disp0` (dan jalur zero-copy scanout pada fase akhir).
5. **Fallback Aman Mutlak:** Jika `GPU=off`, paket `InfernoVgpu.fw` tidak memiliki flag kext+bundle lengkap, atau `inferno-vgpu-server` mati, iOS tetap boot normal dengan `GLES2,0` software fallback tanpa black screen atau SEP panic.

## 4.2 Non-Goals

- Emulasi perangkat keras Apple AGX G12P / RTKit `gfx-asc` asli.
- Hardware video decode/encode (`VideoToolbox` paravirtualization) — ditunda ke PRD terpisah setelah Metal stabil.
- Dukungan versi iOS selain iOS 14.0 (`18A5351d`) sebelum seluruh fase di PRD #2 lulus pada `18A5351d`.
- Modifikasi langsung ke `vendor/metal2vulkan/` (tetap memakai crate publik `metal2vulkan` & `metal2vulkan-runtime` apa adanya).

---

# 5. Arsitektur End-to-End (Guest iOS ↔ Host Vulkan)

```text
+-----------------------------------------------------------------------+
|                       GUEST: iOS 14.0 (t8030)                         |
|                                                                       |
|  [App Metal / SpringBoard / QuartzCore]                               |
|        |                                                              |
|        v                                                              |
|  Metal.framework  --(baca MetalPluginName dari IORegistry)-->         |
|        |                                                              |
|        v                                                              |
|  /System/Library/Extensions/InfernoMetal.bundle (TrustCache CDHash)   |
|    - InfernoMTLDevice (IOGPUMetalDevice / MTLDevice)                  |
|    - InfernoWireClient (per-queue submission/completion ring)         |
|    - IOSurface <-> host texture/buffer sync                           |
|        |  IOConnectMapMemory(Region 2) + IOConnectCallScalarMethod    |
|        v                                                              |
|  InfernoVgpuDriver.kext (diinjeksi iBoot ke kernelcache XNU)          |
|    - InfernoVgpuService : IOGPU / IOAccelerator (IOAcceleratorES)     |
|    - InfernoVgpuUserClient (map 64 MiB shared RAM, doorbell, fence)   |
|    - IOSurface device-cache sync callbacks                            |
|        |  MMIO 0x280000000 / Doorbell 0x280010000 / RAM 0x290000000   |
+--------|--------------------------------------------------------------+
         |  (AppleARMIODevice: arm-io/inferno-vgpu, AIC IRQ 0x210)
         v
+-----------------------------------------------------------------------+
|                EMULATOR: vendor/Inferno (apple-pvgpu)                 |
|  - iBoot: inferno_pvgpu_boot.c (IVGF verify, KC inject, TrustCache)   |
|  - MMIO/Doorbell proxy & shared memfd RAM (0x290000000)               |
|  - Display: apple_displaypipe_v4 (disp0) -> VNC / dmabuf scanout      |
+-----------------------------------------------------------------------+
         |  Unix socket (/run/user/$UID/inferno-gpu.sock) + SCM_RIGHTS
         v
+-----------------------------------------------------------------------+
|               HOST: inferno-vgpu-server (Rust, terpisah)              |
|  - Control worker & per-queue ring workers (no global lock)           |
|  - metal2vulkan (AIR -> SPIR-V, disk pipeline cache)                  |
|  - metal2vulkan-runtime -> Vulkan (RTX 3050 / Intel Iris Xe UMA)      |
+-----------------------------------------------------------------------+
```

---

# 6. Desain Teknis Rinci

## 6.1 Ekstraksi ABI & Simbol iOS 14.0 (`inferno-vgpu-fw`)

Perluas crate `inferno-vgpu/crates/inferno-vgpu-firmware/` dengan dua subcommand baru:

1. **`inferno-vgpu-fw extract-abi <research-kc> -o <out-dir>`**
   - Membaca `LC_SYMTAB` dari `kernelcache.research.iphone12b` (149.571 simbol).
   - Mengekspor:
     - `iogpu_vtables.json`: urutan slot vtable nyata untuk `IOGPU`, `IOGPUDeviceUserClient`, `IOGPUCommandQueue`, `AGXAccelerator`, `IOSurface`, dan `IOService`.
     - `kpi_symbols.toml`: daftar alamat simbol inti yang diperlukan kext (`_IOLog`, `IOService::start`, `IOUserClient::externalMethod`, `IOMemoryDescriptor`, `OSMetaClass`, dll.).
2. **`inferno-vgpu-fw match-release <research-kc> <release-kc> -o <reloc-map.json>`**
   - Mencocokkan fungsi & vtable kunci dari `research` ke `release` (`nsyms=0`) menggunakan pencocokan byte signature instruksi ARM64e + referensi string `__TEXT.__cstring` + urutan vtable `__DATA_CONST.__const`.
   - Menghasilkan tabel relokasi deterministik (atau gagal dengan error eksplisit jika ada simbol yang ambigu/tidak ditemukan — *zero silent guess*).
3. **Ekstraksi `.tbd` Userland (`inferno-vgpu/guest/tools/extract_ios_stubs.py`)**
   - Mengekstrak daftar simbol ekspor dari `Metal.framework`, `IOGPU.framework`, `IOSurface.framework`, `IOKit.framework`, dan `Foundation.framework` pada `dyld_shared_cache_arm64e` di `InfernoData/root` menjadi file `.tbd` untuk cross-linking.

## 6.2 Injeksi Kernelcache & TrustCache di iBoot (`inferno_pvgpu_boot.c`)

Perluas [inferno_pvgpu_boot.c](file:///home/ahmadzanisy/Desktop/inferno-setup/inferno-vgpu/firmware/iboot/inferno_pvgpu_boot.c) dan sinkronkan ke `vendor/Inferno/hw/arm/iboot/`:

### A. Injeksi Kext ke `MH_EXECUTE` Kernelcache di RAM
Saat `t8030_post_kernel_load()` dipanggil (tepat setelah `ck_patch_kernel()` dan sebelum `apple_iboot_boot_xnu()`):

1. Verifikasi paket `IVGF v1` memiliki entry `IVG_FW_ENTRY_KEXT` (`InfernoVgpuDriver.kext`) dengan SHA-256 valid.
2. Periksa ruang alokasi kernelcache di `apple_boot_parse_macho()` (`boot.c`): sediakan *headroom* yang cukup di atas `kc_end` (mis. +4 MiB aligned 16 KiB) saat `pvgpu-fw` aktif agar tidak terjadi buffer overflow saat menyisipkan segmen kext.
3. Peta segmen Mach-O kext (`__TEXT`, `__TEXT_EXEC`, `__DATA_CONST`, `__DATA`) pada alamat virtual 16 KiB-aligned setelah `kc_end`:
   - Jalankan relokasi internal & eksternal kext (termasuk pointer authentication / `arm64e` rebase & bind jika diperlukan pada kernel non-PAC/PAC) menggunakan tabel simbol/binding yang dibundel di entry `IVGF`.
   - Perbarui `__PRELINK_INFO.__info` dengan menyisipkan `<dict>` `InfernoVgpu` sebelum `</array></dict>` (menggunakan XML `OSUnserializeXML` yang sudah dihasilkan oleh `inferno-vgpu-fw stage`).
   - Daftarkan `kmod_info` dan alamat start Mach-O kext ke `__PRELINK_INFO.__kmod_info` dan `__PRELINK_INFO.__kmod_start` (atau salin tabel ke ruang ekstensi & perbarui header section) agar `OSKext::loadPrelinkedKexts()` otomatis memuat dan memanggil `InfernoVgpuService`.
4. Tambahkan opsi `boot-kc-mode`: untuk validasi awal Fase 2B/2C, dukung juga booting langsung dengan `kernelcache.research.iphone12b` agar log kernel `IOLog` & simbol panic mudah dibaca.

### B. Injeksi CDHash ke Static TrustCache di iBoot
Tambahkan entri opsional `IVG_FW_ENTRY_CDHASH` (atau baca langsung dari header Mach-O `LC_CODE_SIGNATURE` milik `IVG_FW_ENTRY_BUNDLE` dalam `InfernoVgpu.fw`):
- Saat `iboot.c` memproses `os->trustcache` (`apple_iboot_load_trustcache_data()`), bangun TrustCache v1 gabungan (entry asli + CDHash 20-byte SHA-256 milik `InfernoMetal.bundle` dan executable uji Metal), lalu urutkan secara leksikografis (syarat wajib binary search TrustCache XNU).
- Hasilnya: `AMFI` dan Library Validation iOS 14.0 mengizinkan `SpringBoard`, `backboardd`, dan app sistem memuat `/System/Library/Extensions/InfernoMetal.bundle` tanpa perlu hook MAC policy seperti `RgfxGate`.

### C. Promosi `graphics-featureset-class` yang Aman
Tetap pertahankan guard yang sudah ada di `inferno_pvgpu_fw_apply_dt()`:
- `product/graphics-featureset-class` hanya dinaikkan dari `"GLES2,0"` ke `"APPLE6"` (dengan `graphics-featureset-fallbacks = "APPLE5:APPLE4:APPLE3:APPLE2:APPLE1:GLES2,0"`) **hanya jika** seluruh syarat terpenuhi:
  1. `IVG_FW_FLAG_ENABLE_METAL_FEATURESET | IVG_FW_FLAG_HAS_KEXT | IVG_FW_FLAG_HAS_BUNDLE` semua aktif.
  2. Injeksi kext ke kernelcache berhasil (`err == 0`).
  3. `apple_pvgpu_is_connected(pvgpu) == true`.
- Jika salah satu syarat gagal, iBoot otomatis mempertahankan `"GLES2,0"` sehingga emulator tetap masuk Home Screen dengan software rendering.

## 6.3 Guest Kernel Driver (`InfernoVgpuDriver.kext`)

Evolusi dari [InfernoVgpuDriver.cpp](file:///home/ahmadzanisy/Desktop/inferno-setup/inferno-vgpu/guest/driver/src/InfernoVgpuDriver.cpp):

1. **Tahap 2C (Transport & Probe Minimal — Subclass `IOService`)**:
   - Match ke `AppleARMIODevice` dengan `IONameMatch = ["inferno-vgpu", "apple,inferno-pvgpu"]`.
   - Memetakan `getDeviceMemoryWithIndex(0..2)` → verifikasi `regs[IVG_REG_MAGIC/4] == 0x49564750` dan `IVG_STATUS_HOST_READY`.
   - Menginisialisasi control ring pada offset awal Region 2, mengirim `IVG_OP_PING` (`cookie = 0x1AFE2000`) + doorbell write ke Region 1, menunggu completion, dan mencetak `IOLog("InfernoVgpuService: host GPU '%s' online (ping ok)\n", ...)` ke konsol serial `main.log`.
   - Mendaftarkan `IOInterruptEventSource` pada IRQ index 0 (`0x210`) untuk membangunkan thread yang menunggu completion tanpa busy-wait.
2. **Tahap 2D/2E (Accelerator Penuh — Subclass `IOGPU` / `IOAcceleratorES`)**:
   - Mempublikasikan properti IORegistry wajib:
     - `IOMatchCategory = "IOAcceleratorES"`
     - `MetalPluginName = "InfernoMetal"`
     - `MetalPluginClassName = "InfernoMTLDevice"`
     - `IOGLESBundleName = "AppleMetalGLRenderer"`
     - `IOUserClientClass = "InfernoVgpuUserClient"`
   - Mengekspor `IOUserClient` untuk:
     - `clientMemoryForType(0)`: mapping Region 2 (64 MiB shared RAM, write-combine / cached sesuai mode UMA).
     - `externalMethod(kDoorbell)`: menulis doorbell queue ID ke Region 1.
     - `externalMethod(kImportSurface)` / `kReleaseSurface`: mengambil `IOSurface` dari `IOSurfaceID`, mengunci backing memory, dan mendaftarkan callback sinkronisasi device-cache saat CPU melakukan `IOSurfaceLock`/`Unlock`.
     - Handler *device-lost*: jika IRQ `IVG_IRQ_ERROR` atau `IVG_STATUS_DEVICE_LOST` terdeteksi, menolak submission baru dengan `kIOReturnNotReady` tanpa kernel panic.

## 6.4 Guest Metal Bundle (`InfernoMetal.bundle`)

Evolusi dari [InfernoMetal.mm](file:///home/ahmadzanisy/Desktop/inferno-setup/inferno-vgpu/guest/bundle/src/InfernoMetal.mm):

1. **Instalasi ke RootFS via `inferno-fs-patcher`**:
   - Perluas opsi `--pvgpu-bundle <path>` di [main.cpp](file:///home/ahmadzanisy/Desktop/inferno-setup/vendor/Inferno/fspatcher/main.cpp) dari sekadar validasi menjadi **instalasi atomik & revertible** ke `/System/Library/Extensions/InfernoMetal.bundle/` (menyimpan `Info.plist` + binary Mach-O `InfernoMetal` dengan permission `0755`), serta mengaktifkan mode `--pvgpu` pada `CoreImagePatches` & `QuartzCorePatches` (tidak men-nullify `AccelServer::renderer` dan tidak mematikan `_CI_ENABLE_METAL_GPU`).
2. **Protokol Metal & SPI Privat iOS 14.0**:
   - Mengimplementasikan kelas `InfernoMTLDevice` (sesuai `MetalPluginClassName`) yang membuka `IOServiceMatching("InfernoVgpuService")`.
   - Memetakan `hasUnifiedMemory` langsung dari `IvgDeviceCaps` host (`true` di Intel Iris Xe → zero-shadow untuk Shared buffer; `false` di NVIDIA RTX 3050 → dua salinan dengan sinkronisasi `didModifyRange:` / `synchronizeResource:`).
   - Implementasi `newTextureWithDescriptor:iosurface:plane:` untuk `CAMetalLayer` dan komposisi `QuartzCore`.
   - Mode diagnostik `INFERNO_METAL_LOG_SPI=1` mencatat setiap selector privat (`respondsToSelector:` / `forwardInvocation:`) yang diminta `QuartzCore`/`SpringBoard` agar seluruh SPI iOS 14.0 dapat ditutup satu per satu tanpa crash senyap.

## 6.5 Presentasi Display (`IOSurface` → `disp0` → Zero-Copy)

1. **Tahap 2E (Stabil & Langsung Tampil):**
   - `CAMetalLayer` di iOS menyerahkan `IOSurface` drawable ke `InfernoMetal.bundle`.
   - Saat `presentDrawable:` / `commit`, hasil render pada `VkImage` di host disinkronkan ke backing memory `IOSurface` (`IVG_OP_TEXTURE_READBACK` / staging ring), sehingga `IOMobileFramebuffer` (`disp0` / `apple_displaypipe_v4`) otomatis menampilkan frame ke VNC `127.0.0.1:5900` tanpa mengubah pipeline display Inferno.
2. **Tahap 2F (Zero-Copy Scanout):**
   - Server mengekspor frame terkomposisi lewat `IVG_IPC_READ_DISPLAY` atau `dma-buf` FD ke QEMU console (`dpy_gl_scanout_dmabuf`), menghindari salinan piksel lewat CPU TCG guest untuk `SpringBoard`.

---

# 7. Strategi Build Toolchain (`arm64e-apple-ios14.0`)

Mengacu langsung pada pengalaman `reims-vgpu` (*"reims-ctl adds the set to an image, since a Linux host cannot build it"*), proyek mendukung **dua jalur build**:

| Jalur | Lingkungan | Kelebihan | Keterbatasan |
|---|---|---|---|
| **Jalur A (Utama / Recommended)** | macOS + Xcode (`iPhoneOS14.0.sdk` + Kernel/IOKit headers) | Kompilasi `arm64e` + Objective-C++ ARC + `codesign` 100% resmi; biner dipaketkan ke `InfernoVgpu.fw` lalu dijalankan di Linux | Membutuhkan akses mesin macOS/CI macOS saat mengubah kode guest |
| **Jalur B (Eksperimental Linux-Native)** | Linux host dengan `clang` + `ld64.lld` + `.tbd` hasil ekstrak `dyld_shared_cache` + minimal IOKit/IOGPU C++ headers dari `kernelcache.research.iphone12b` + `ldid` | Iterasi penuh 100% di Linux tanpa macOS | Membutuhkan pemeliharaan header IOKit/Obj-C runtime minimal & verifikasi ABI PAC `arm64e` secara ketat |

Kedua jalur menghasilkan output paket tunggal yang identik:
`inferno-vgpu/firmware/build/InfernoVgpu.fw` (format `IVGF v1`, terverifikasi SHA-256 oleh `inferno-vgpu-fw verify` dan `inferno_pvgpu_boot.c`).

---

# 8. Fase Implementasi & Exit Criteria

| Fase | Ruang Lingkup | Artefak Utama | Exit Criteria (Terukur) |
|---|---|---|---|
| **2A. ABI & Symbol Extraction** | Tambah `extract-abi` & `match-release` di `inferno-vgpu-fw`; ekstrak `.tbd` dari `InfernoData/root` | `iogpu_vtables.json`, `kpi_symbols.toml`, `reloc_map.json`, stub `.tbd` | 100% simbol KPI & vtable `IOGPU`/`IOSurface` terpetakan di `research` & terverifikasi di `release` KC tanpa mismatch |
| **2B. iBoot Kext & TrustCache Injector** | Implementasi injeksi segmen Mach-O + `__PRELINK_INFO` nyata & TrustCache CDHash merger di `inferno_pvgpu_boot.c` + `boot.c` | `inferno_pvgpu_boot.c`, `boot.c`, `iboot.c`, unit test `inferno-vgpu-firmware` | Boot iOS 14.0 dengan dummy/minimal kext di `InfernoVgpu.fw` sukses sampai Home Screen tanpa kernel panic; CDHash terbaca di TrustCache |
| **2C. Live Kext Probe & Ring Ping** | Compile & inject `InfernoVgpuDriver.kext` (`IOService` + `IOUserClient`) yang memetakan `arm-io/inferno-vgpu` | `InfernoVgpuDriver.kext` di dalam `InfernoVgpu.fw` | `main.log` mencetak `InfernoVgpuService: attached to NVIDIA GeForce RTX 3050 / Intel Iris Xe (ping ok)` dan `inferno-vgpu-server` menerima record `IVG_OP_PING` nyata dari guest |
| **2D. Metal Bundle Compute & Render** | Upgrade kext ke `IOAcceleratorES` (`MetalPluginName=InfernoMetal`); pasang `InfernoMetal.bundle` via `inferno-fs-patcher --pvgpu-bundle` | `InfernoMetal.bundle`, `inferno-fs-patcher` write support | Program uji di iOS memanggil `MTLCreateSystemDefaultDevice()` (non-nil), menjalankan compute shader & render segitiga offscreen di GPU host dengan hasil pixel cocok 100% |
| **2E. `IOSurface` & `SpringBoard` Compositing** | Implementasi `IOSurface` device-cache sync + `CAMetalLayer` + SPI privat `QuartzCore` (`product/graphics-featureset-class = APPLE6`) | Full `IOGPU` / `IOSurface` sync + patch SPI | `SpringBoard` & Settings / app bawaan merender via Metal di GPU host (`GPU_DEVICE=NVIDIA` & `GPU_DEVICE=Intel`), tanpa black screen atau loop crash `backboardd` |
| **2F. Zero-Copy & Stabilitas** | Jalur scanout `dma-buf`, tuning mode UMA Intel Iris Xe (`unified=true`) vs discrete RTX 3050, uji ketahanan *device-lost* | Scanout zero-copy + benchmark & soak test | Uji 1 jam stabil; mematikan `inferno-vgpu-server` di tengah render menghasilkan device-lost bersih tanpa kernel/SEP panic; FPS terukur lebih tinggi dari `GPU=off` |

---

# 9. Pengujian & Validasi

1. **Host & Firmware Gate (`cargo xtask check` + `make -C inferno-vgpu firmware`):**
   - Verifikasi parser Mach-O, relokasi, builder/verifier `IVGF`, dan C11 header ABI.
   - Uji Vulkan live di **kedua GPU host** (`INFERNO_VGPU_DEVICE=NVIDIA` dan `INFERNO_VGPU_DEVICE=Intel`) lewat `cargo test -p inferno-vgpu -p inferno-vgpu-vulkan -- --ignored`.
2. **Kernel & IORegistry Verification (Live Guest):**
   - `idevicediagnostics ioregentry InfernoVgpuService` menampilkan service aktif dengan `IOMatchCategory = IOAcceleratorES`, `MetalPluginName = InfernoMetal`, dan `IOUserClientClass = InfernoVgpuUserClient`.
   - `main.log` menampilkan log sukses `IVG_OP_PING` + `IVG_OP_GET_DEVICE_CAPS` dari XNU.
3. **Metal Correctness & Visual Verification:**
   - Binary uji `arm64e` (compute vector-add, triangle render pass, texture upload/readback, `IOSurface` roundtrip) dijalankan di guest dan dibandingkan bit-exact terhadap referensi `metal2vulkan`.
   - Verifikasi VNC framebuffer non-hitam (`vncview.py`) saat `SpringBoard` aktif.
4. **Resilience & Fallback Tests:**
   - `kill -9` pada `inferno-vgpu-server` saat render aktif → guest menerima `device-lost`, QEMU & SEP tetap hidup.
   - `GPU=off ./run.sh all` → kembali ke jalur framebuffer standar tanpa regresi.

---

# 10. Risiko Utama & Mitigasi

| Risiko | Dampak | Mitigasi |
|---|---|---|
| Perbedaan layout vtable / pointer authentication (`arm64e` PAC) pada `IOGPUFamily` iOS 14.0 | Kernel panic saat kext memanggil superclass `IOGPU` | Ekstrak vtable langsung dari `kernelcache.research.iphone12b`; validasi awal pada `research` KC sebelum porting signature ke `release` KC |
| `SpringBoard` / `QuartzCore` memanggil private selector Metal yang belum diimplementasikan | `backboardd` crash loop / black screen saat `APPLE6` diaktifkan | Guard `IVG_FW_FLAG_ENABLE_METAL_FEATURESET` (default tetap `GLES2,0` selama pengembangan) + logging `INFERNO_METAL_LOG_SPI=1` |
| `inferno-fs-patcher` merusak sealed APFS root volume saat menulis `InfernoMetal.bundle` | Boot gagal / panic APFS | Gunakan mekanisme transaksi & backup revertible milik `inferno-fs-patcher` (`-r` untuk rollback instan) + uji `--dry-run` dulu |
| Overhead readback `IOSurface` di Fase 2E pada GPU discrete (NVIDIA) | FPS UI terbatas sebelum zero-copy | Uji awal pada Intel Iris Xe (`unified=true`) untuk latensi rendah, lalu aktifkan `dma-buf` scanout di Fase 2F |
| Ketergantungan SDK Apple (`iPhoneOS14.0.sdk`) di host Linux | Menghambat iterasi lokal | Sediakan paket `InfernoVgpu.fw` prebuilt (Jalur A) dan eksplorasi cross-linker `clang` + `ld64.lld` + `.tbd` (Jalur B) di Fase 2A |
