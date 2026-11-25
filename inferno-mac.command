#!/bin/bash
#
# Inferno for macOS (.command = double-clickable from Finder).
# All logic lives here; setup-mac.command / run-mac.command / accel-check.command
# are thin launchers.
#
# Tested target: macOS 12+ (Intel or Apple Silicon). No Linux kernel modules,
# no udev, no systemctl, no pacman/apt.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "$SCRIPT_DIR/config.env"

ROOT_DIR="$SCRIPT_DIR/InfernoData"
INFERNO_DIR="$SCRIPT_DIR/Inferno"
SSS_DIR="$SCRIPT_DIR/SimulatedSigningServer"
LOCAL_PREFIX="${LOCAL_PREFIX:-$SCRIPT_DIR/.local}"
WORK_DIR="$SCRIPT_DIR/.build-deps"
LOG_DIR="${INFERNO_LOG_DIR:-${TMPDIR:-/tmp}/inferno-setup-$UID}"
ACCEL_FILE="$ROOT_DIR/.accel"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
USB_PORT="8030"
SSS_PORT="${SSS_PORT:-8031}"
RESTORE_MARKER="$ROOT_DIR/.restored"

# macOS: no LD_LIBRARY_PATH; use DYLD_* and MacPorts paths (/opt/local).
PORT_BIN="$(command -v port 2>/dev/null || true)"
PORT_PREFIX="${MACPORTS_PREFIX:-}"
if [ -z "$PORT_PREFIX" ] && [ -n "$PORT_BIN" ]; then
    PORT_PREFIX="$(cd "$(dirname "$PORT_BIN")/.." && pwd)"
fi
if [ -z "$PORT_PREFIX" ] && [ -d /opt/local ]; then
    PORT_PREFIX=/opt/local
fi

export PATH="$LOCAL_PREFIX/bin:$HOME/.local/bin${PORT_PREFIX:+:$PORT_PREFIX/bin}:$PATH"
export CPATH="$LOCAL_PREFIX/include${PORT_PREFIX:+:$PORT_PREFIX/include}${CPATH:+:$CPATH}"
export LIBRARY_PATH="$LOCAL_PREFIX/lib${PORT_PREFIX:+:$PORT_PREFIX/lib}${LIBRARY_PATH:+:$LIBRARY_PATH}"
export DYLD_LIBRARY_PATH="$LOCAL_PREFIX/lib${PORT_PREFIX:+:$PORT_PREFIX/lib}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="$LOCAL_PREFIX/lib/pkgconfig:$LOCAL_PREFIX/share/pkgconfig${PORT_PREFIX:+:$PORT_PREFIX/lib/pkgconfig:$PORT_PREFIX/share/pkgconfig}${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info() { printf '%b\n' "${CYAN}[INFO]${NC} $1"; }
ok()   { printf '%b\n' "${GREEN}[OK]${NC} $1"; }
warn() { printf '%b\n' "${YELLOW}[WARN]${NC} $1"; }
err()  { printf '%b\n' "${RED}[ERR]${NC} $1" >&2; }
die()  { err "$1"; exit 1; }

# ---------------------------------------------------------------- host / accel

# Physical arch vs the arch we actually build for. Rosetta 2 reports x86_64
# to uname/xmake even on Apple Silicon, so HVF is unusable in that Terminal.
host_arch_native() { uname -m; }

is_rosetta() {
    [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]
}

build_arch() { host_arch_native; }

hv_support() { sysctl -n kern.hv_support 2>/dev/null || echo 0; }

# auto -> hvf on native arm64 macOS with HV support, else tcg.
chosen_accel() {
    local want="${ACCEL:-auto}"
    case "$want" in
        auto)
            if [ "$(build_arch)" = "arm64" ] && [ "$(hv_support)" = "1" ]; then
                echo hvf
            else
                echo tcg
            fi
            ;;
        hvf|tcg) echo "$want" ;;
        *) die "ACCEL must be auto, hvf, or tcg (got: $want)" ;;
    esac
}

validate_accel() {
    local accel="$1"
    if [ "$accel" = "hvf" ]; then
        [ "$(build_arch)" = "arm64" ] || die "HVF needs a native arm64 build; this Terminal/xmake is $(build_arch)."
        [ "$(hv_support)" = "1" ] || die "kern.hv_support is not 1 on this Mac."
    fi
}

describe_accel() {
    local accel
    accel="$(chosen_accel)"
    validate_accel "$accel"
    info "Host: $(sw_vers -productName 2>/dev/null || echo macOS) $(sw_vers -productVersion 2>/dev/null || echo '?') / $(host_arch_native)"
    if is_rosetta; then
        warn "Terminal ini jalan di bawah Rosetta 2 — xmake akan build x86_64."
        warn "Untuk HVF, buka Terminalnative (encer Terminal.app/iTerm, uncheck 'Open using Rosetta')."
    fi
    info "kern.hv_support = $(hv_support) (Hypervisor.framework tersedia untuk guest x86_64 di Intel)"
    if [ "$(build_arch)" != "arm64" ]; then
        warn "Inferno meng-compile hvf hanya di host macosx+arm64 (xmake/options.lua)."
        warn "Guest iPhone 11 adalah ARM64; HVF di Intel tidak bisa menjalankan guest ARM64."
        info "Akselerator yang dipakai: $accel (emulasi software, JIT). Ini batasan platform, bukan bug config."
    else
        info "Host arm64 terdeteksi; HVF bisa menjalankan guest ARM64 langsung di CPU (accel terpilih: $accel)."
    fi
    info "Accel terpilih untuk build & run: $accel (set ACCEL=tcg|hvf|auto untuk mengubah)"
    if [ -f "$ACCEL_FILE" ]; then
        info "Accel tersimpan di ${ACCEL_FILE#"$SCRIPT_DIR"/}: $(cat "$ACCEL_FILE")"
    fi
}

# ---------------------------------------------------------------- prerequisites

port_cmd() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        sudo "$@"
    fi
}

ensure_xmake() {
    if ! command -v xmake >/dev/null 2>&1; then
        [ -n "$PORT_BIN" ] || die "xmake tidak ada dan 'port' tidak ditemukan. Pasang MacPorts, lalu: sudo port install xmake"
        info "xmake belum ada; installing lewat MacPorts ..."
        port_cmd port install xmake || die "sudo port install xmake gagal."
        hash -r
    fi
    command -v xmake >/dev/null 2>&1 || die "xmake installation failed."
    local ver major
    ver="$(xmake --version 2>/dev/null | head -n 1 | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -n 1 || true)"
    major="${ver%%.*}"
    if [ -n "$ver" ] && [ "${major:-0}" -lt 3 ] 2>/dev/null; then
        die "Inferno butuh xmake >= 3.0 (ketemu $ver). Update: sudo port upgrade xmake"
    fi
    ok "xmake: $(command -v xmake) ${ver:-}"
}

install_deps() {
    [ -n "$PORT_BIN" ] || die "MacPorts ('port') tidak ditemukan. Pasang MacPorts dulu: https://www.macports.org"
    info "Installing dependencies via MacPorts (sudo port install) ..."
    [ "${PORT_SELFUPDATE:-0}" = "1" ] && port_cmd port selfupdate
    port_cmd port install \
        xmake cmake pkgconfig wget git \
        glib2 nettle gnutls libtasn1 lz4 gmp \
        libpng libyuv zstd libssh libusb
    hash -r
    if ! pkg-config --exists nettle 2>/dev/null || \
       [ "$(pkg-config --modversion nettle 2>/dev/null || echo 0)" \< "3.10" ]; then
        warn "nettle < 3.10; coba: sudo port upgrade nettle"
    fi
    ok "MacPorts dependencies siap (prefix: ${PORT_PREFIX:-?})"
}

ensure_lzfse() {
    if [ -f "$LOCAL_PREFIX/include/lzfse.h" ] || pkg-config --exists lzfse 2>/dev/null; then
        ok "lzfse headers found"
        return 0
    fi
    info "Build lzfse ke $LOCAL_PREFIX ..."
    mkdir -p "$WORK_DIR"
    if [ ! -d "$WORK_DIR/lzfse/.git" ]; then
        git clone --depth 1 https://github.com/lzfse/lzfse "$WORK_DIR/lzfse"
    fi
    cmake -S "$WORK_DIR/lzfse" -B "$WORK_DIR/lzfse-build" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$LOCAL_PREFIX" \
        -DCMAKE_INSTALL_LIBDIR=lib \
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5
    cmake --build "$WORK_DIR/lzfse-build" --parallel "$JOBS"
    cmake --install "$WORK_DIR/lzfse-build"
    [ -f "$LOCAL_PREFIX/include/lzfse.h" ] || die "lzfse headers missing after install."
    ok "lzfse built"
}

clone_sources() {
    if [ ! -f "$INFERNO_DIR/xmake.lua" ]; then
        if [ -f "$SCRIPT_DIR/.gitmodules" ]; then
            info "Init Inferno submodule ..."
            ( cd "$SCRIPT_DIR" && git submodule update --init --recursive Inferno )
        else
            info "Clone Inferno (repo lama GitHub sudah diarsipkan) ..."
            git clone --depth 1 https://git.chefkiss.dev/AppleHax/Inferno.git "$INFERNO_DIR"
        fi
    fi
    if [ ! -f "$SSS_DIR/xmake.lua" ]; then
        info "Clone SimulatedSigningServer ..."
        git clone --depth 1 https://git.chefkiss.dev/AppleHax/SimulatedSigningServer.git "$SSS_DIR"
    fi
    ok "Source trees ready"
}

xmake_configure() {
    local accel="$1" hvf=n tcg=n
    [ -f "$INFERNO_DIR/xmake.lua" ] || die "Inferno source missing. Jalankan: $0 clone"
    if [ "$accel" = "hvf" ]; then hvf=y; else tcg=y; fi
    # Memilih satu accel juga membuat default di system/vl.c tidak ambigu:
    # kalau tcg ikut ke-build, Inferno akan default ke tcg walaupun hvf ada.
    info "xmake configure: -m release --tcg=$tcg --hvf=$hvf"
    ( cd "$INFERNO_DIR" && xmake f -y -m release --qom_cast_debug=n --wxwidgets=n --tcg="$tcg" --hvf="$hvf" )
}

sync_accel_config() {
    local accel
    accel="$(chosen_accel)"
    validate_accel "$accel"
    mkdir -p "$ROOT_DIR"
    if [ ! -f "$ACCEL_FILE" ] || [ "$(cat "$ACCEL_FILE" 2>/dev/null)" != "$accel" ]; then
        info "Accel berubah -> configure ulang xmake"
        printf '%s\n' "$accel" > "$ACCEL_FILE"
        xmake_configure "$accel"
    fi
}

build_inferno() {
    ensure_xmake
    ensure_lzfse
    clone_sources
    local accel
    accel="$(chosen_accel)"
    validate_accel "$accel"
    xmake_configure "$accel"
    printf '%s\n' "$accel" > "$ACCEL_FILE"
    info "Build Inferno (accel=$accel, jobs=$JOBS) ..."
    ( cd "$INFERNO_DIR" && xmake -y -j "$JOBS" )
    info "Build SimulatedSigningServer ..."
    ( cd "$SSS_DIR" && xmake f -y && xmake -y )
    ok "Build selesai (inferno-aarch64, inferno-img, inferno-fs-patcher, SSS)"
}

create_disks() {
    mkdir -p "$ROOT_DIR"
    [ -f "$INFERNO_DIR/xmake.lua" ] || die "Inferno source missing."
    local disk size spec
    for spec in \
        "root:32G" "firmware:8M" "syscfg:128K" "ctrl_bits:8K" "nvram:8K" \
        "effaceable:4K" "panic_log:1M" "sep_nvram:64K" "sep_ssc:128K"; do
        disk="${spec%%:*}"
        size="${spec##*:}"
        [ -f "$ROOT_DIR/$disk" ] && { info "  $disk sudah ada"; continue; }
        ( cd "$INFERNO_DIR" && xmake r inferno-img create -f raw "$ROOT_DIR/$disk" "$size" )
    done
    ok "Disk images ready"
}

prepare_firmware() {
    mkdir -p "$ROOT_DIR"
    local rom="${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1}"
    if [ ! -s "$ROOT_DIR/$rom" ] && [ -n "${SEP_ROM_URL:-}" ]; then
        info "Download SEP ROM ..."
        curl -fL --progress-bar -o "$ROOT_DIR/$rom" "$SEP_ROM_URL"
    fi
    [ -s "$ROOT_DIR/$rom" ] && ok "SEP ROM: $rom" || warn "SEP ROM missing: $ROOT_DIR/$rom"
    [ -n "${SEP_IV_KEY:-}" ] && ok "SEP IV+Key configured" || warn "SEP_IV_KEY kosong"
}

fetch_ipsw() {
    local name="${IPSW_FILENAME:-}" url="${IPSW_URL:-}"
    [ -n "$name" ] && [ -n "$url" ] || die "IPSW_FILENAME/IPSW_URL belum di-set."
    mkdir -p "$ROOT_DIR"
    [ -s "$ROOT_DIR/$name" ] && { ok "IPSW sudah ada"; return 0; }
    warn "Download IPSW (~5.5 GB) ..."
    curl -fL -C - --progress-bar -o "$ROOT_DIR/$name" "$url"
    ok "IPSW downloaded"
}

setup_all() {
    describe_accel
    install_deps
    build_inferno
    create_disks
    prepare_firmware
    [ "${SKIP_IPSW:-0}" = "1" ] || fetch_ipsw
    ok "Setup selesai. Jalankan: ./run-mac.command main"
}

# ---------------------------------------------------------------- run / verify

require_built() {
    [ -f "$INFERNO_DIR/xmake.lua" ] || die "Inferno source missing. Jalankan: ./setup-mac.command"
    command -v xmake >/dev/null 2>&1 || die "xmake missing. Jalankan: ./setup-mac.command"
    [ -f "$ACCEL_FILE" ] || die "Accel belum dipilih. Jalankan: ./setup-mac.command (atau ./inferno-mac.command build)"
}

find_binary() {
    find "$INFERNO_DIR/build" -type f -name inferno-aarch64 -perm -u+x 2>/dev/null | head -n 1
}

# Boot singkat dengan -S (paused) untuk membuktikan -accel diterima binary.
smoke_accel() {
    local accel="$1" bin log="$LOG_DIR/accel-$accel.log" pid i
    require_built
    bin="$(find_binary)"
    [ -n "$bin" ] || die "Binary inferno-aarch64 belum ada. Jalankan build dulu."
    mkdir -p "$LOG_DIR"
    info "Smoke test: $(basename "$bin") -accel $accel -M ${BOARD:-t8030} -S ..."
    ( cd "$INFERNO_DIR" && "$bin" -accel "$accel" \
        -M "${BOARD:-t8030}" -m 128M -S \
        -display none -vnc none -serial none ) >"$log" 2>&1 </dev/null &
    pid=$!
    for i in $(seq 1 12); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -TERM "$pid" 2>/dev/null || true
        sleep 1
        kill -KILL "$pid" 2>/dev/null || true
        ok "-accel $accel diterima (proses hidup sampai di-stop smoke test)."
    else
        wait "$pid" 2>/dev/null || true
        warn "-accel $accel keluar lebih awal; cek $log"
    fi
    printf '%s\n' "--- log (tail) ---"
    grep -Ei 'accel|hvf|kvm|tcg|hypervisor|error|warning' "$log" 2>/dev/null | tail -n 20 || true
}

accel_check() {
    describe_accel
    local accel
    accel="$(chosen_accel)"
    validate_accel "$accel"
    if [ -f "$ACCEL_FILE" ]; then
        info "Build terkonfigurasi untuk: $(cat "$ACCEL_FILE")"
        [ "$(cat "$ACCEL_FILE")" = "$accel" ] || warn "Config build beda dengan accel terpilih — jalankan build ulang."
    else
        warn "Belum ada build; jalankan ./setup-mac.command dulu."
    fi
    if [ -n "$(find_binary)" ]; then
        smoke_accel "$accel"
    else
        warn "Binary belum ada, smoke test dilewati."
    fi
    info "Kesimpulan: di Mac $(host_arch_native) ini, guest ARM64 Inferno memakai $(chosen_accel)."
}

main_vm() {
    require_built
    local accel display_args=()
    accel="$(cat "$ACCEL_FILE")"
    validate_accel "$accel"
    check_files
    [ -n "${DISPLAY_ARGS:-}" ] && display_args=(-display "$DISPLAY_ARGS")
    info "Boot iPhone (accel=$accel, log: $LOG_DIR/main.log) ..."
    cd "$INFERNO_DIR"
    exec xmake r inferno-aarch64 \
        -accel "$accel" \
        -M "${BOARD:-t8030},sep-rom=$ROOT_DIR/${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1},kaslr-off=${KASLR_OFF:-true}" \
        -append "${KERNEL_ARGS:-tlto_us=-1 mtxspin=-1 serial=3 wdt=-1}" \
        -smp "${MAIN_SMP:-7}" -m "${MAIN_RAM:-4G}" \
        "${display_args[@]}" \
        -serial mon:stdio \
        -drive file="$ROOT_DIR/sep_nvram",if=pflash,format=raw \
        -drive file="$ROOT_DIR/sep_ssc",if=pflash,format=raw \
        -drive file="$ROOT_DIR/root",format=raw,if=none,id=root \
        -device nvme-ns,drive=root,bus=nvme-bus.0,nsid=1,nstype=1,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/firmware",format=raw,if=none,id=firmware \
        -device nvme-ns,drive=firmware,bus=nvme-bus.0,nsid=2,nstype=2,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/syscfg",format=raw,if=none,id=syscfg \
        -device nvme-ns,drive=syscfg,bus=nvme-bus.0,nsid=3,nstype=3,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/ctrl_bits",format=raw,if=none,id=ctrl_bits \
        -device nvme-ns,drive=ctrl_bits,bus=nvme-bus.0,nsid=4,nstype=4,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/nvram",if=none,format=raw,id=nvram \
        -device apple-nvram,drive=nvram,bus=nvme-bus.0,nsid=5,nstype=5,id=nvram,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/effaceable",format=raw,if=none,id=effaceable \
        -device nvme-ns,drive=effaceable,bus=nvme-bus.0,nsid=6,nstype=6,logical_block_size=4096,physical_block_size=4096 \
        -drive file="$ROOT_DIR/panic_log",format=raw,if=none,id=panic_log \
        -device nvme-ns,drive=panic_log,bus=nvme-bus.0,nsid=7,nstype=8,logical_block_size=4096,physical_block_size=4096
}

check_files() {
    local missing=0 f
    for f in root firmware syscfg ctrl_bits nvram effaceable panic_log sep_nvram sep_ssc "${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1}"; do
        [ -s "$ROOT_DIR/$f" ] || { warn "Missing/empty: $f"; missing=1; }
    done
    [ "$missing" -eq 0 ] || die "Disk/ROM belum lengkap. Jalankan ./setup-mac.command"
}

signing_server() {
    [ -f "$SSS_DIR/xmake.lua" ] || die "SSS missing. Jalankan: ./setup-mac.command"
    cd "$SSS_DIR"
    exec xmake r SimulatedSigningServer "$SSS_HOST" "$SSS_PORT"
}

port_open() { nc -z -w 1 127.0.0.1 "$1" >/dev/null 2>&1; }

start_sss_bg() {
    port_open "$SSS_PORT" && { ok "SSS sudah listen di $SSS_PORT"; return 0; }
    mkdir -p "$LOG_DIR"
    info "Start signing server di $SSS_PORT ..."
    nohup "$0" sss >"$LOG_DIR/sss.log" 2>&1 </dev/null &
    local i
    for i in $(seq 1 40); do port_open "$SSS_PORT" && { ok "SSS up"; return 0; }; sleep 0.5; done
    die "SSS gagal start; cek $LOG_DIR/sss.log"
}

start_vm_bg() {
    mkdir -p "$LOG_DIR"
    if pgrep -x inferno-aarch64 >/dev/null 2>&1; then ok "Emulator sudah jalan"; return 0; fi
    info "Boot emulator di background (log: $LOG_DIR/main.log) ..."
    nohup "$0" main >"$LOG_DIR/main.log" 2>&1 </dev/null &
    local i
    for i in $(seq 1 40); do
        port_open "$USB_PORT" && { ok "Emulator up di port $USB_PORT"; return 0; }
        pgrep -x inferno-aarch64 >/dev/null 2>&1 || break
        sleep 0.5
    done
    die "Emulator gagal start; cek $LOG_DIR/main.log"
}

run_all() {
    describe_accel
    require_built
    check_files
    start_sss_bg
    start_vm_bg
    info "Emulator jalan. Buka layar: ./run-mac.command view (VNC 127.0.0.1:5900)"
    warn "USB/restore di macOS butuh VirtualHere client GUI + idevicerestore."
    if [ "${SKIP_RESTORE:-0}" != "1" ] && [ ! -f "$RESTORE_MARKER" ]; then
        warn "Belum ada marker restore. Jalankan restore manual setelah USB device ter-attach."
    fi
}

view_screen() {
    if command -v vncviewer >/dev/null 2>&1; then
        exec vncviewer 127.0.0.1:5900
    fi
    info "Membuka Screen Sharing ke VNC 127.0.0.1:5900 ..."
    open "vnc://127.0.0.1:5900"
}

fs_patch() {
    require_built
    ( cd "$INFERNO_DIR" && xmake r inferno-fs-patcher -d "$ROOT_DIR/root" )
}

stop_all() {
    info "Stopping Inferno services (SIGTERM dulu agar SEP konsisten) ..."
    pkill -TERM -x inferno-aarch64 2>/dev/null || true
    pkill -TERM -x SimulatedSigningServer 2>/dev/null || true
    pkill -TERM -x vhclient 2>/dev/null || true
    sleep 2
    pkill -KILL -x inferno-aarch64 2>/dev/null || true
    pkill -KILL -x SimulatedSigningServer 2>/dev/null || true
    ok "Stopped."
}

usage() {
    cat <<EOF
Usage: $0 [perintah]

  accel-check   Cek host + akselerator (disarankan pertama kali)
  setup         MacPorts deps + clone + build + disks + firmware + IPSW
  deps          Hanya install dependencies via sudo port
  clone         Clone Inferno & SimulatedSigningServer
  build         Configure + build dengan accel terpilih
  disks         Buat 9 disk image di InfernoData/
  firmware      Cek/download SEP ROM
  ipsw          Download IPSW
  main          Jalankan emulator di foreground (pakai -accel eksplisit)
  all           Start SSS + emulator di background
  view          Buka layar iOS lewat Screen Sharing/VNC
  fs-patch      Patch filesystem setelah restore
  stop          Stop semua service

Env: ACCEL=auto|hvf|tcg, SKIP_IPSW=1, DISPLAY_ARGS=..., MAIN_SMP=..., MAIN_RAM=...
Catatan: di host Intel, guest ARM64 hanya bisa TCG (HVF/KVM di Inferno arm64-only).
EOF
}

menu() {
    while true; do
        printf '\n===== Inferno macOS =====\n'
        printf 'Host: %s / accel: %s\n\n' "$(host_arch_native)" "$(chosen_accel 2>/dev/null || echo '?')"
        printf '  1) Cek akselerasi\n  2) Setup penuh\n  3) Build ulang\n  4) Jalankan emulator\n'
        printf '  5) Start semua (background)\n  6) Lihat layar\n  7) Stop semua\n  0) Keluar\n'
        printf 'Pilih: '
        local choice
        read -r choice || exit 0
        case "$choice" in
            1) accel_check ;;
            2) setup_all ;;
            3) build_inferno ;;
            4) main_vm ;;
            5) run_all ;;
            6) view_screen ;;
            7) stop_all ;;
            0) exit 0 ;;
            *) warn "Pilihan tidak dikenal: $choice" ;;
        esac
    done
}

cmd="${1:-menu}"
case "$cmd" in
    accel-check|accel) accel_check ;;
    setup|all-setup)   setup_all ;;
    deps)              install_deps ;;
    clone)             ensure_xmake; ensure_lzfse; clone_sources ;;
    build)             build_inferno ;;
    disks)             create_disks ;;
    firmware)          prepare_firmware ;;
    ipsw)              fetch_ipsw ;;
    main)              main_vm ;;
    sss|signing-server) signing_server ;;
    all)               run_all ;;
    view)              view_screen ;;
    fs-patch)          fs_patch ;;
    stop|kill)         stop_all ;;
    help|-h|--help)    usage ;;
    menu)              menu ;;
    *)                 usage; exit 1 ;;
esac
