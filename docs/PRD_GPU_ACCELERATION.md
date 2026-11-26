# Inferno GPU Acceleration (metal2vulkan)

## Product Requirements Document & Technical Design

**Project:** inferno-setup (ChefKiss Inferno, t8030 / iPhone 11)
**Goal:** Metal in the emulated iOS runs on the host GPU through Vulkan
**Translator:** `~/Desktop/ZDE-minimal/ThirdParty/metal2vulkan` (Rust)
**Related:** ZDE `Docs/PRD_AI_QEMU#2.md` sections 7.6 and 10.1
**Status:** Proposed / research

---

# 1. Ringkasan

Saat ini Inferno menjalankan iOS 14 tanpa GPU. Machine `t8030` tidak punya model GPU Apple (AGX); yang ada hanya display pipe (`hw/display/apple_displaypipe_v4.c`), yang menyalin framebuffer ke console QEMU dan VNC (`127.0.0.1:5900`). Semua render di guest dikerjakan oleh CPU yang diemulasikan TCG, jadi UI lambat dan app yang butuh Metal tidak jalan.

Target dokumen ini: **full acceleration**. Panggilan Metal di guest diteruskan ke host, diterjemahkan oleh metal2vulkan ke Vulkan, dijalankan di GPU host, dan hasilnya tampil di layar tanpa salinan lewat CPU guest.

**USB tidak terlibat di jalur grafis.** USB (VirtualHere 8030, usbmuxd) hanya untuk restore dan layanan perangkat. GPU memakai perangkat virtual tersendiri.

# 2. Kondisi sekarang

```text
iOS app / SpringBoard
   -> QuartzCore / Metal  (tidak ada GPU -> jalur software / gagal)
   -> IOMobileFramebuffer
   -> apple_displaypipe_v4 (QEMU)
   -> QEMU console -> VNC 5900 -> vncview.py
```

| Komponen | Lokasi | Catatan |
|---|---|---|
| Machine | `Inferno/hw/arm/t8030.c` | Tidak ada node/perangkat GPU yang aktif |
| Display | `Inferno/hw/display/apple_displaypipe_v4.c` | Scanout framebuffer saja |
| Filesystem patch | `Inferno/fspatcher/` | Sudah bisa mem-patch dylib/framework di root image; dipakai untuk injeksi shim |
| Viewer | `vncview.py` | GTK + cairo, scaling |

# 3. Tujuan dan non-goals

## 3.1 Goals

1. Aplikasi Metal di guest (compute dan render) berjalan di GPU host.
2. CoreAnimation/SpringBoard dikomposisi lewat Metal yang diakselerasi, sehingga UI lancar.
3. Hasil render tampil langsung dari VkImage host ke window, tanpa readback ke memori guest (zero-copy pada fase akhir).
4. Kegagalan fitur Metal yang belum didukung menghasilkan error yang jelas di guest dan log host, bukan hasil yang salah.
5. Bisa dimatikan: `GPU=off` tetap memakai jalur framebuffer yang sekarang.

## 3.2 Non-goals (fase awal)

- Emulasi penuh GPU AGX (command stream hardware Apple). Hanya riset (lihat 5.3).
- OpenGL ES di guest (sudah deprecated di iOS; bisa menyusul lewat Metal).
- Multi-GPU, VR, video decode hardware.
- Dukungan iOS selain versi yang dipakai `config.env` (iOS 14.0 18A5351d) sebelum fase 6.

# 4. Arsitektur

```text
GUEST (iOS di Inferno)                          HOST (Linux)
----------------------                          ------------
App / QuartzCore
   |
Metal.framework  -- diganti/di-interpose oleh -->
libInfernoMetal.dylib (shim, di-inject fs-patcher)
   |  serialisasi panggilan Metal -> command stream
   v
IOKit user client  (driver guest minimal)
   |  ring buffer + doorbell + IRQ
   v
=========== apple-pvgpu (perangkat QEMU baru) ===========
                                                   |
                                     vhost-user socket / shared memory
                                                   v
                                 inferno-gpu-server  (proses terpisah, Rust)
                                    |-- decoder command stream
                                    |-- metal2vulkan-runtime (Metal API di Vulkan)
                                    |-- metal2vulkan (AIR -> SPIR-V, pipeline cache)
                                    `-- presenter: VkImage -> dma-buf
                                                   |
                                 QEMU console (dmabuf scanout) -> window / VNC
```

## 4.1 Komponen

| # | Komponen | Bahasa | Lokasi usulan | Tugas |
|---|---|---|---|---|
| 1 | `libInfernoMetal.dylib` | C / Obj-C (arm64 iOS) | `inferno-gpu/guest/shim/` | Implementasi `MTLDevice`, `MTLCommandQueue`, dll. di guest; serialisasi ke protokol |
| 2 | Driver guest | C++ (IOKit) | `inferno-gpu/guest/driver/` | Expose ring buffer dan doorbell ke userspace lewat IOUserClient |
| 3 | `apple-pvgpu` | C (QEMU) | `Inferno/hw/display/apple_pvgpu.c` | Perangkat MMIO: BAR ring, doorbell, IRQ, device tree node; meneruskan ke server |
| 4 | Protokol | Spesifikasi + header C | `inferno-gpu/protocol/` | Format perintah, objek, fence, versi |
| 5 | `inferno-gpu-server` | Rust | `metal2vulkan/crates/metal2vulkan-driver` (bin) | Decode protokol, panggil metal2vulkan-runtime, kelola resource dan fence |
| 6 | Presenter | Rust + C (QEMU) | server + `ui/` QEMU | Export hasil sebagai dma-buf ke console QEMU |

## 4.2 Kenapa proses terpisah (vhost-user) untuk renderer

- **Isolasi crash:** panic di driver Vulkan atau metal2vulkan tidak mematikan QEMU, sehingga state SEP tidak rusak (penyebab SEP panic yang pernah terjadi adalah QEMU mati mendadak).
- **Bahasa:** server tetap Rust murni tanpa FFI ke QEMU; QEMU tetap C.
- **Lisensi:** Inferno GPL-3.0/AGPL-3.0; metal2vulkan GPL-2.0+ OR LGPL-3.0+. Keduanya kompatibel dengan GPL-3.0, tetapi proses terpisah membuat batasnya jelas, dan server yang sama bisa dipakai ulang oleh ZDE.
- **Pola yang sudah ada:** sama seperti `vhost-user-gpu` di QEMU upstream.

## 4.3 Protokol (garis besar)

- Shared memory ring (submission dan completion), satu per command queue.
- Objek diidentifikasi handle 64-bit yang dibuat guest dan dipetakan di server.
- Perintah dasar: `CreateDevice`, `CreateBuffer`, `CreateTexture`, `CreateLibrary(metallib bytes)`, `CreatePipeline`, `Encode*`, `Commit`, `Present(surface, texture)`, `Wait(fence)`.
- Data buffer besar tidak disalin: buffer guest dipetakan sebagai memori bersama (blob resource), lalu diimpor ke Vulkan (`VK_EXT_external_memory_host` atau buffer staging).
- Versi protokol dinegosiasikan saat `CreateDevice`; kemampuan device host (dari metal2vulkan) dilaporkan ke guest sebagai feature set Metal.
- Typed refusal metal2vulkan dikirim balik sebagai error terstruktur (nama fitur Metal + alasan).

# 5. Opsi sisi guest

## 5.1 Opsi B: shim Metal paravirtual (dipilih)

Metal API di guest diganti `libInfernoMetal.dylib`. Karena CoreAnimation di iOS 14 memakai Metal untuk komposisi, mengakselerasi Metal ikut mempercepat seluruh UI.

Cara injeksi: perluas `inferno-fs-patcher` agar memasang dylib dan meng-interpose `MTLCreateSystemDefaultDevice` / `MTLCopyAllDevices` (atau mengganti binary `Metal.framework` di shared cache).

## 5.2 Driver guest: pertanyaan terbuka (Phase 0)

Shim userspace butuh jalan ke perangkat `apple-pvgpu`. Kandidat:

| Kandidat | Kelebihan | Risiko |
|---|---|---|
| a. Kext baru di kernelcache | Bersih, IOUserClient standar | Harus menyuntik kext ke kernelcache dan menangani signing/KTRR di emulator |
| b. Menumpang driver yang sudah ada (mis. kanal IOMFB/display pipe) | Tidak perlu kext baru | Rapuh, terikat versi iOS |
| c. MMIO langsung ke userspace lewat patch kernel | Paling sederhana untuk prototipe | Tidak aman, hanya untuk riset |

Keputusan diambil di akhir Phase 0 berdasarkan prototipe kecil (doorbell ping-pong).

## 5.3 Opsi C: emulasi AGX (riset saja)

Meniru GPU Apple asli supaya driver AGX bawaan iOS dan Metal asli berjalan tanpa diubah, lalu menerjemahkan command stream AGX ke Vulkan. Butuh reverse engineering firmware ASC/RTKit GPU dan format perintah (rujukan: Asahi Linux, yang menargetkan M1/M2, bukan A13). Tidak masuk jalur utama.

# 6. Display dan presentasi

1. **Tahap 1:** `Present` di server melakukan readback dan menulis ke framebuffer display pipe. Lambat, tapi membuktikan jalur end-to-end.
2. **Tahap 2:** server mengekspor VkImage sebagai dma-buf; QEMU men-scanout lewat `dpy_gl_scanout_dmabuf` (butuh `-display gtk,gl=on` atau `egl-headless` + VNC).
3. `vncview.py` tetap berguna untuk `egl-headless`; untuk akselerasi penuh, display GTK/SDL QEMU dengan GL dipakai langsung.

# 7. Integrasi inferno-setup

`config.env`:

```bash
# off | pv
GPU="off"
GPU_SERVER_SOCKET="/run/user/$UID/inferno-gpu.sock"
GPU_LOG="${TMPDIR:-/tmp}/opencode/gpu.log"
```

`run.sh`:

- `./run.sh gpu`: menjalankan `inferno-gpu-server` di background.
- `run.sh main`: jika `GPU=pv`, tambahkan `-chardev socket,id=gpu,path=$GPU_SERVER_SOCKET -device apple-pvgpu,chardev=gpu`.
- `run.sh all`: menyalakan server GPU sebelum emulator; jika server mati, emulator tetap jalan dengan framebuffer.

`setup.sh`:

- `./setup.sh gpu`: build `inferno-gpu-server` (cargo) dan shim guest (cross-compile arm64-apple-ios), lalu `fs-patch` untuk memasang shim.

# 8. Fase dan kriteria selesai

| Fase | Isi | Exit criteria |
|---|---|---|
| 0. Feasibility | Pilih driver guest (5.2); prototipe doorbell guest ke QEMU ke server | Satu perintah ping dari userspace iOS sampai ke server dan dibalas |
| 1. Device dan protokol | `apple-pvgpu`, spesifikasi protokol v0, server skeleton | Buffer dibuat dan diisi guest, dibaca balik dari server |
| 2. Compute | Shim: device, buffer, library, compute pipeline | Kernel compute Metal sederhana di guest memberi hasil benar dari GPU host |
| 3. Render offscreen | Texture, render pass, draw | Segitiga dirender dan dibaca balik dengan pixel yang benar |
| 4. Present | `Present` lewat readback ke display pipe | App Metal tampil di layar emulator |
| 5. CoreAnimation | QuartzCore memakai device paravirtual | SpringBoard dikomposisi lewat GPU host; FPS terukur lebih tinggi dari baseline |
| 6. Zero-copy dan stabilitas | dma-buf scanout, pipeline cache di disk, fence/timeline | Uji 1 jam tanpa crash; tanpa readback CPU |

# 9. Pengujian

- **Protokol:** unit test encode/decode di Rust; fuzzing decoder server (input dari guest dianggap tidak tepercaya).
- **Kebenaran:** gunakan `metal2vulkan-validation` sebagai referensi; kasus yang sama dijalankan via shim di guest dan dibandingkan.
- **Integrasi:** app uji Metal di guest (compute, triangle, texture sampling) dijalankan otomatis setelah boot.
- **Performa:** waktu boot sampai SpringBoard, FPS scroll, latensi frame; bandingkan `GPU=off` dan `GPU=pv`.
- **Ketahanan:** matikan server di tengah render; guest harus mendapat error (device lost), dan QEMU serta SEP tidak ikut crash.

# 10. Risiko

| Risiko | Dampak | Mitigasi |
|---|---|---|
| Driver guest sulit disuntik (kernelcache, signing) | Blokir semua fase | Phase 0 khusus untuk ini; opsi 5.2c untuk prototipe |
| Metal privat/SPI yang dipakai QuartzCore tidak terdokumentasi | Fase 5 tertunda | Logging panggilan dari shim untuk memetakan API yang benar-benar dipakai |
| Fitur Metal yang belum didukung metal2vulkan | App gagal | Typed refusal plus laporan per-app; prioritaskan fitur dari log |
| Overhead serialisasi | FPS rendah | Batching perintah, ring tanpa lock, blob memory |
| Crash renderer mematikan QEMU | SEP panic | Proses terpisah (4.2); QEMU memperlakukan server hilang sebagai device lost |
| Perubahan upstream Inferno | Konflik patch | Perangkat baru di file sendiri; perubahan `t8030.c` sekecil mungkin |

# 11. Keputusan

| Area | Keputusan |
|---|---|
| Pendekatan | Shim Metal paravirtual (opsi B) |
| Translator | metal2vulkan, tetap Rust |
| Renderer host | Proses terpisah `inferno-gpu-server` via vhost-user |
| Perangkat QEMU | `apple-pvgpu` baru, MMIO dan ring |
| Presentasi | Readback dulu, lalu dma-buf zero-copy |
| USB | Tidak dipakai untuk grafis |
| Fallback | `GPU=off` (framebuffer seperti sekarang) |
| Reuse | Server yang sama dipakai ZDE (`Graphics/Backends/Metal`) |
