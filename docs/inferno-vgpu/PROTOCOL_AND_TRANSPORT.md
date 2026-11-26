# Wire Protocol & Transport Spec — `inferno-vgpu`

Dokumen ini mendefinisikan protokol biner antara guest iOS (`guest/bundle` + `guest/driver`), perangkat emulator (`apple_pvgpu.c`), dan host Rust (`crates/inferno-vgpu-graphics`). Desainnya mengadopsi protokol `reims-graphics` (`reims_graphics_wire.h`) yang telah teruji performa dan akurasinya dengan `metal2vulkan`.

---

## 1. Pilihan Transport Hardware di `t8030` (Inferno)

Berbeda dengan macOS x86_64 yang selalu menggunakan PCIe BAR, mesin `t8030` (Apple A13) pada Inferno mendukung dua opsi transport:

| Opsi | Implementasi di `t8030.c` | Kelebihan | Kekurangan |
|---|---|---|---|
| **Opsi 1: SysBus MMIO (Direkomendasikan untuk Phase 0–3)** | Node baru di Apple Device Tree (`arm-io/inferno-vgpu`) dengan 3 region MMIO + 1 IRQ line | Tidak perlu enumerasi PCIe/DART IOMMU; sangat mudah di-map dari driver iOS | Non-standar dibanding `reims-graphics-pci` |
| **Opsi 2: PCIe di `apcie`** | Device PCIe (`1234:5247`) di bawah `apcie/bridge*` (`hw/pci-host/apcie.c`) | Hampir identik 1:1 dengan `reims-graphics-pci.c` milik `reims-vgpu` | Harus menangani `dart-apcie` IOMMU & konfigurasi PCIe iOS |

**Prinsip penting:** Apa pun transport fisik yang dipilih (SysBus MMIO atau PCIe BAR), **layout register, Shared Region, dan Doorbell di dalamnya tetap sama persis** sehingga crate Rust `inferno-vgpu-graphics` tidak perlu berubah.

---

## 2. Peta Memori & Register Device

### 2.1 Region 0 — Control Registers (MMIO, 4 KiB / 16 KiB)
| Offset | Ukuran | Akses | Nama | Fungsi |
|---|---|---|---|---|
| `0x000` | u32 | RO | `MAGIC` | `0x49564750` (`"IVGP"`) |
| `0x004` | u32 | RO | `VERSION` | Versi protokol (mayor/minor) |
| `0x008` | u64 | RO | `FEATURES` | Bitmask fitur host (`SURFACE_PAGES`, `SHARED_POOL`, `DOORBELL_PAGES`, `UNIFIED_MEMORY`, dll.) |
| `0x010` | u32 | RW | `STATUS` | `0` = reset device, `1` = driver ready, `2` = host ready |
| `0x018` | u64 | RW | `CONTROL_RING_OFFSET` | Offset ring 0 (control) di dalam Region 1 (Shared Region) |
| `0x020` | u32 | WO | `CONTROL_DOORBELL` | Bangunkan control thread host (ring 0) |
| `0x024` | u32 | WO | `QUEUE_DOORBELL` | Tulis `queue_id` untuk membangunkan worker queue tertentu |
| `0x028` | u32 | RW | `IRQ_STATUS` / `IRQ_ACK` | Status & acknowledge interrupt completion |
| `0x030` | u64 | RO | `REGION_SIZE` | Ukuran total Shared Region (Region 1) |
| `0x038` | u64 | RO | `POOL_BASE` / `POOL_SIZE` | Range alamat fisik Shared Memory Pool (untuk buffer `Shared`) |

### 2.2 Region 1 — Shared Region RAM (Host `memfd`, default 256 MiB – 512 MiB)
Dipetakan sebagai RAM bersama antara guest dan host:
- **Ring 0 (Control Ring):** submission + completion untuk operasi manajemen (create/destroy context, buat queue, alokasi blok, display scanout).
- **Per-Queue Rings (Ring 1..N):** satu pasangan submission/completion ring per `MTLCommandQueue`.
- **Command Buffer Bodies & Staging Chunks:** payload command encoder dan upload `.metallib` (AIR).

### 2.3 Region 2 — Doorbell Pages (Opsional, Optimasi Fase Lanjut)
- Blok halaman MMIO (tiap halaman 16 KiB sesuai page size arm64 iOS) yang di-bind per-context agar userspace iOS bisa menulis doorbell secara langsung tanpa syscall ke kernel setiap commit.

---

## 3. Struktur Ring Buffer (Lockless SPSC)

Setiap ring adalah pasangan **Submission Ring** (guest producer → host consumer) dan **Completion Ring** (host producer → guest consumer):

```c
// Didefinisikan di Rust (inferno-vgpu-graphics) dan di-export via cbindgen
typedef struct __attribute__((aligned(64))) {
    uint32_t producer;      // Ditulis producer (release store di arm64)
    uint32_t consumer;      // Ditulis consumer (release store di arm64)
    uint32_t capacity_mask; // (power_of_two - 1)
    uint32_t flags;         // bit 0: HOST_IDLE (guest hanya bunyikan doorbell jika bit ini 1)
    uint64_t data_offset;   // Offset byte payload ring di Shared Region
} IvgRingHeader;
```

**Aturan Ordering (ARM64 iOS):**
- Guest menulis seluruh payload record → `stlr` (store-release) ke `producer` → cek `flags & HOST_IDLE` → jika idle, tulis `QUEUE_DOORBELL`.
- Host membaca `producer` (Acquire) → proses record → tulis completion record → update `consumer` (Release) → naikkan IRQ hanya jika guest meng-arm interrupt.
- Tidak ada VM exit per submit ketika ring sedang sibuk (*adaptive polling*).

---

## 4. Model Objek, Handle, & Address Pinning

1. **Context Isolation:** Setiap proses iOS yang membuka device mendapat `context_id` (u32). Semua objek lokal berada di namespace `context_id` tersebut.
2. **Handle 64-bit:** Guest mengalokasikan `obj_id` (u64) secara lokal tanpa round-trip ke host.
3. **Address & Resource ID Pinning:**
   - Guest memilih `gpuAddress` (buffer) dan `gpuResourceID` (texture/sampler/pipeline) dari slice alamat yang diberikan host saat inisialisasi context.
   - Host mem-pin alamat tersebut di address space internal `metal2vulkan` sehingga **Argument Buffers** bekerja tanpa perlu patching.
4. **Cross-Context Sharing (`IOSurface` & `MTLSharedEvent`):**
   - Objek lintas proses (seperti frame dari app iOS ke `SpringBoard` / `QuartzCore`) diidentifikasi lewat global `surface_id` / `global_name`.

---

## 5. Daftar Opcode Inti (Tahap 1 → Penuh)

### 5.1 Control Ring (Ring 0)
- `OP_PING` (Phase 0: verifikasi end-to-end doorbell & completion)
- `OP_GET_DEVICE_CAPS` (host melaporkan `DeviceCaps` dari `metal2vulkan`: GPU family, limits, supported pixel formats, `hasUnifiedMemory`)
- `OP_CONTEXT_CREATE` / `OP_CONTEXT_DESTROY`
- `OP_QUEUE_CREATE` / `OP_QUEUE_DESTROY`
- `OP_BLOCK_ALLOC` / `OP_BLOCK_FREE` (alokasi blok memori di Shared Pool / VRAM)
- `OP_LIBRARY_CREATE` (kirim SHA-256 + bytes `.metallib` AIR jika belum ada di cache host)
- `OP_PIPELINE_CREATE_COMPUTE` / `OP_PIPELINE_CREATE_RENDER` (dengan function constants)
- `OP_SURFACE_IMPORT` / `OP_SURFACE_RELEASE` (registrasi `IOSurface` guest pages)
- `OP_DISPLAY_SET_MODE` / `OP_DISPLAY_PRESENT` (scanout frame ke display)

### 5.2 Queue Submission Ring (Ring 1..N)
- `CMD_BUFFER_BEGIN` / `CMD_BUFFER_COMMIT` (dengan `seqno` monoton)
- **Blit Encoder:** `BLIT_COPY_BUFFER`, `BLIT_COPY_TEXTURE`, `BLIT_FILL_BUFFER`, `BLIT_GEN_MIPMAPS`, `BLIT_SYNC_RESOURCE`
- **Compute Encoder:** `COMPUTE_SET_PIPELINE`, `COMPUTE_SET_BUFFER`, `COMPUTE_SET_TEXTURE`, `COMPUTE_SET_BYTES`, `COMPUTE_DISPATCH_THREADS`, `COMPUTE_DISPATCH_THREADGROUPS`
- **Render Encoder:** `RENDER_BEGIN_PASS`, `RENDER_SET_PIPELINE`, `RENDER_SET_VIEWPORT`, `RENDER_SET_SCISSOR`, `RENDER_SET_VERTEX_BUFFER`, `RENDER_SET_FRAGMENT_TEXTURE`, `RENDER_DRAW_PRIMITIVES`, `RENDER_DRAW_INDEXED`, `RENDER_END_PASS`
- **Sync:** `EVENT_SIGNAL`, `EVENT_WAIT`, `FENCE_UPDATE`, `FENCE_WAIT`

### 5.3 Completion Record
```c
typedef struct {
    uint64_t seqno;
    uint32_t status;        // 0 = Completed, non-zero = MTLCommandBufferError
    uint32_t refusal_code;  // Kode refusal terstruktur dari metal2vulkan (0 jika sukses)
    uint64_t gpu_start_ns;
    uint64_t gpu_end_ns;
} IvgCompletionRecord;
```

---

## 6. Alur Shader AIR → SPIR-V

1. Aplikasi iOS memuat `.metallib` (atau memanggil `newLibraryWithSource:` yang dikompilasi Metal compiler guest ke AIR).
2. `libInfernoMetal.dylib` menghitung **SHA-256** dari payload AIR dan mengirim `OP_LIBRARY_CREATE(sha256, bytes_offset, len)`.
3. Host Rust mengecek cache SHA-256:
   - **Cache hit:** langsung kembalikan handle library.
   - **Cache miss:** serahkan bytes AIR ke `metal2vulkan` → menghasilkan SPIR-V + reflection → simpan di pipeline cache disk & memori.
4. Saat `OP_PIPELINE_CREATE_*` diterima (beserta `MTLFunctionConstantValues` yang sudah di-resolve oleh guest bundle), `metal2vulkan-runtime` membuat `VkPipeline` di GPU host.
