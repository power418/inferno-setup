# Integrasi Sisi Guest iOS 14 (`t8030`) — `inferno-vgpu`

Dokumen ini menjelaskan bagaimana guest iOS 14.0 (`18A5351d`) pada Inferno mengenali GPU virtual, meneruskan panggilan Metal ke `inferno-vgpu`, dan menampilkan hasilnya ke layar.

---

## 1. Perbedaan Guest macOS (`reims-vgpu`) vs Guest iOS 14 (`inferno-vgpu`)

| Komponen Guest | Di `reims-vgpu` (macOS) | Di `inferno-vgpu` (iOS 14 Inferno) |
|---|---|---|
| **Metal Implementation** | `ReimsGraphics.bundle` (Obj-C++ Metal plugin yang dimuat `Metal.framework`) | `libInfernoMetal.dylib` / `InfernoMetal.bundle` (adaptasi dari `guest/bundle/` untuk `arm64-apple-ios14.0`) |
| **Kernel Driver** | `ReimsGraphics.kext` + `RgfxGate.kext` + `Graft.kext` via OpenCore/Ambrosia | Driver transport minimal (diputuskan di Phase 0: kext di kernelcache, piggyback driver eksisting, atau MMIO userspace bridge) |
| **Display Stack** | `IOFramebuffer` / `AppleGraphicsDeviceControl` (AGDC) | `IOMobileFramebuffer` (`apple_displaypipe_v4`) + `IOSurface` + `QuartzCore` |
| **Code Signing & Injection** | `RgfxGate.kext` (AMFI hook) | `vendor/Inferno/fspatcher` (patch `dyld_shared_cache` & rootfs) + patch AMFI Inferno |

---

## 2. `guest/bundle/` — Implementasi Metal Guest (`libInfernoMetal.dylib`)

`guest/bundle/` pada `reims-vgpu` sudah memiliki hampir seluruh implementasi kelas Metal dalam Obj-C++/C++ (`RgfxBuffer.mm`, `RgfxTexture.mm`, `RgfxCommandObjects.mm`, `RgfxEncoder.cpp`, `RgfxComputePipelineState.mm`, `RgfxRenderPipelineState.mm`, `RgfxFunction.mm`, `RgfxHeap.mm`, `RgfxEvent.mm`, `RgfxFence.mm`, `RgfxArgumentEncoder.mm`).

### Adaptasi yang Diperlukan untuk iOS 14:
1. **Target Kompilasi:** `arm64-apple-ios14.0` (dan `arm64e` jika binary dimuat langsung ke proses sistem `arm64e` seperti `SpringBoard` / `backboardd`).
2. **Pemangkasan macOS-only:**
   - Hapus `RgfxGL*` (OpenGL desktop tidak ada di iOS).
   - Hapus `MTLStorageModeManaged` dari eksposur default iOS (iOS 14 hanya mengenal `Shared`, `Private`, `Memoryless`; secara internal di host, `Shared` tetap dipetakan ke Shared Pool host-cached).
   - Sesuaikan `supportsFamily:` agar melaporkan **Apple GPU Family 5/6 (A13)** sesuai kemampuan yang didukung `metal2vulkan-runtime`.
3. **Interpose / Registrasi `MTLDevice`:**
   - Pastikan `MTLCreateSystemDefaultDevice()` dan `MTLCopyAllDevices()` mengembalikan instance `InfernoMTLDevice`.
   - Implementasikan private SPI Metal yang dipanggil oleh `QuartzCore` / `CoreImage` / `UIKit` di iOS 14 (dipetakan secara inkremental menggunakan logging di shim saat Phase 5).
4. **Dukungan `IOSurface`:**
   - Saat `QuartzCore` memanggil `newTextureWithDescriptor:iosurface:plane:`, shim mengambil `IOSurfaceID` + alamat/backing pages, mengirim `OP_SURFACE_IMPORT` ke host, dan membungkusnya sebagai `MTLTexture` yang bisa langsung di-render dan di-present oleh host.

---

## 3. `guest/driver/` — Transport Userspace ↔ Perangkat `apple_pvgpu`

Userspace iOS (`libInfernoMetal.dylib`) membutuhkan akses ke **Region 0 (MMIO Doorbell)** dan **Region 1 (Shared Region RAM)**. Di Phase 0, pilih salah satu dari 3 jalur berikut:

| Opsi | Mekanisme | Kelebihan | Risiko | Rekomendasi |
|---|---|---|---|---|
| **A. Kext IOKit di Kernelcache** | Inject kext minimal (`InfernoVgpu.kext`) yang match node Device Tree `arm-io/inferno-vgpu` dan mengekspos `IOUserClient` (`IOConnectMapMemory` untuk Shared Region & Doorbell) | Paling bersih, multi-proses aman, mirip `guest/kext/` di `reims-vgpu` | Harus patch/rebuild kernelcache iOS 14 & lolos KTRR/signing di emulator | **Target jangka panjang (Phase 2+)** |
| **B. Piggyback Driver Eksisting** | Menumpang kanal `IOUserClient` milik `IOMobileFramebuffer` / display pipe | Tidak perlu inject kext baru | Rapuh & sangat terikat binary iOS 14.0 | Cadangan jika Opsi A terblokir |
| **C. Direct MMIO / Shared Memory Bridge via Kernel Patch** | Patch kecil di kernel/emulator untuk memetakan fisik `apple_pvgpu` ke proses tertentu | Tercepat untuk membuktikan doorbell ping-pong | Tidak aman untuk produksi | **Direkomendasikan khusus untuk Phase 0 (Feasibility)** |

---

## 4. Injeksi ke RootFS iOS 14 via `fspatcher`

ChefKiss Inferno sudah memiliki `vendor/Inferno/fspatcher/` yang mem-patch `dyld_shared_cache_arm64e` dan filesystem root image (`InfernoData/root`).

Alur injeksi `guest/fspatch/`:
1. Salin `libInfernoMetal.dylib` ke `/usr/lib/libInfernoMetal.dylib` (atau `/System/Library/Extensions/InfernoMetal.bundle`) di root image guest.
2. Tambahkan rule di `fspatcher` untuk:
   - Mem-patch `MTLCreateSystemDefaultDevice` di `Metal.framework` (dalam `dyld_shared_cache`) agar memanggil entry point `InfernoMTLCreateSystemDefaultDevice` di `libInfernoMetal.dylib`, **atau**
   - Mengarahkan `DYLD_INSERT_LIBRARIES` / `MetalPluginName` pada `launchd.plist` untuk proses target (`SpringBoard`, `backboardd`, dan app uji).
3. Sediakan flag agar `GPU=off` dapat melewati patch ini atau membuat shim langsung fallback tanpa error.

---

## 5. Jalur Display & Presentasi iOS

### Tahap 1 (Phase 4 — Kompatibilitas VNC):
```text
App / SpringBoard -> Metal render ke IOSurface texture (Vulkan VkImage di host)
  -> Present -> host readback ke framebuffer apple_displaypipe_v4
  -> QEMU console -> VNC :5900 -> vncview.py
```
- Membuktikan seluruh jalur render end-to-end dengan infrastruktur tampilan `inferno-setup` yang sudah ada.

### Tahap 2 (Phase 6 — Full Zero-Copy Acceleration):
```text
App / SpringBoard -> Metal render ke IOSurface texture (Vulkan VkImage di host)
  -> Present -> inferno-vgpu-window (SDL3 + Vulkan swapchain langsung)
                ATAU export VkImage sebagai dma-buf -> QEMU GTK/SDL GL scanout
```
- Tidak ada readback ke CPU guest maupun host; target 60 FPS pada resolusi native iPhone 11 (`828 × 1792`).
