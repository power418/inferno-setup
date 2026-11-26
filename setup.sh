#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/config.env"

ROOT_DIR="$SCRIPT_DIR/InfernoData"
if [ -f "$SCRIPT_DIR/vendor/Inferno/xmake.lua" ]; then
    INFERNO_DIR="$SCRIPT_DIR/vendor/Inferno"
else
    INFERNO_DIR="$SCRIPT_DIR/Inferno"
fi
VGPU_DIR="$SCRIPT_DIR/inferno-vgpu"
SSS_DIR="$SCRIPT_DIR/SimulatedSigningServer"
WORK_DIR="$SCRIPT_DIR/.build-deps"
LOCAL_PREFIX="${LOCAL_PREFIX:-$SCRIPT_DIR/.local}"
JOBS="${JOBS:-$(nproc)}"
LOG_DIR="${INFERNO_LOG_DIR:-${TMPDIR:-/tmp}/inferno-setup-$UID}"

# Local tools are visible to xmake/gcc without root.
export PATH="$LOCAL_PREFIX/bin:$HOME/.local/bin:$PATH"
export CPATH="$LOCAL_PREFIX/include${CPATH:+:$CPATH}"
export LIBRARY_PATH="$LOCAL_PREFIX/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$LOCAL_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="$LOCAL_PREFIX/lib/pkgconfig:$LOCAL_PREFIX/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
err()   { echo -e "${RED}[ERR]${NC} $1" >&2; }
die()   { err "$1"; exit 1; }

ROOT_PREFIX=()
if [ "$(id -u)" -ne 0 ]; then
    # sudoers drop-in /etc/sudoers.d/99-inferno-setup allows these exact
    # commands without a prompt, so the one-shot flow stays non-interactive.
    if command -v sudo >/dev/null 2>&1; then
        ROOT_PREFIX=(sudo -n)
    else
        die "sudo is required for installing system packages."
    fi
fi

detect_distro() {
    local id="unknown" like=""
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        id="${ID:-unknown}"
        like="${ID_LIKE:-}"
    fi
    echo "$id $like"
}

version_ge() {
    # returns 0 if $1 >= $2 (Debian versions with -N suffix are tolerated)
    [ "$(printf '%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}

install_system_deps() {
    info "Installing system dependencies (sudo may ask for your password)..."
    local distro like
    read -r distro like <<<"$(detect_distro)"
    case "$distro $like" in
        *arch*|*cachyos*|*artix*|*manjaro*|*endeavouros*)
            local -a pkgs=(
                base-devel cmake git wget curl unzip pkgconf xmake \
                glib2 libtasn1 lz4 gmp nettle libpng libyuv zstd \
                gnutls libssh libusb usbmuxd python python-pyasn1 python-pyasn1-modules
            )
            # zlib-ng-compat provides zlib but Conflicts With/Replaces zlib.
            # Prefer it on fresh installs; never request both at once.
            if pacman -Qq zlib-ng-compat >/dev/null 2>&1 || \
               pacman -Qq zlib >/dev/null 2>&1; then
                :
            elif pacman -Si zlib-ng-compat >/dev/null 2>&1; then
                pkgs+=(zlib-ng-compat)
            else
                pkgs+=(zlib)
            fi
            "${ROOT_PREFIX[@]}" pacman -S --noconfirm --needed "${pkgs[@]}"
            ;;
        *debian*|*ubuntu*|*pop*|*mint*)
            "${ROOT_PREFIX[@]}" apt-get update -y
            "${ROOT_PREFIX[@]}" apt-get install -y \
                build-essential cmake git wget curl unzip pkg-config xmake \
                libglib2.0-dev zlib1g-dev libtasn1-6-dev liblz4-dev libgmp-dev \
                nettle-dev libpng-dev libyuv-dev libzstd-dev libgnutls28-dev \
                libssh-dev libusb-1.0-0-dev usbmuxd \
                python3 python3-pip python3-pyasn1 python3-pyasn1-modules
            ;;
        *)
            die "Unsupported distro ($distro). Install dependencies manually."
            ;;
    esac

    # xmake requires nettle >= 3.10.
    local nettle_ver
    nettle_ver="$(pkg-config --modversion nettle 2>/dev/null || echo 0)"
    if ! version_ge "$nettle_ver" "3.10"; then
        warn "Nettle $nettle_ver is too old, building 3.10.2 into $LOCAL_PREFIX ..."
        mkdir -p "$WORK_DIR"
        [ -f "$WORK_DIR/nettle-3.10.2.tar.gz" ] || \
            wget -q -O "$WORK_DIR/nettle-3.10.2.tar.gz" \
                https://ftp.gnu.org/gnu/nettle/nettle-3.10.2.tar.gz
        tar -C "$WORK_DIR" -xf "$WORK_DIR/nettle-3.10.2.tar.gz"
        (
            cd "$WORK_DIR/nettle-3.10.2"
            ./configure --prefix="$LOCAL_PREFIX" --disable-shared
            make -j"$JOBS"
            make install
        )
        export PKG_CONFIG_PATH="$LOCAL_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH}"
    fi
    ok "System dependencies installed"
}

ensure_xmake() {
    if command -v xmake >/dev/null 2>&1; then
        ok "xmake: $(command -v xmake)"
        return
    fi
    info "xmake not found, installing to ~/.local ..."
    curl -fsSL https://xmake.io/shget.text | bash
    export PATH="$HOME/.local/bin:$PATH"
    command -v xmake >/dev/null 2>&1 || die "xmake installation failed."
    ok "xmake: $(command -v xmake)"
}

ensure_lzfse() {
    if [ -f "$LOCAL_PREFIX/include/lzfse.h" ] || pkg-config --exists lzfse 2>/dev/null; then
        ok "lzfse headers found"
        return
    fi
    info "Building lzfse into $LOCAL_PREFIX ..."
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

ensure_vnc_viewer() {
    if command -v vncviewer >/dev/null 2>&1 || [ -x "$LOCAL_PREFIX/bin/vncviewer" ]; then
        ok "VNC viewer ready"
        return
    fi
    info "Installing TigerVNC viewer into $LOCAL_PREFIX ..."
    local distro like
    read -r distro like <<<"$(detect_distro)"
    case "$distro $like" in
        *arch*|*cachyos*|*artix*|*manjaro*|*endeavouros*)
            mkdir -p "$WORK_DIR/vnc-pkgs" "$LOCAL_PREFIX"
            local url pkg urls
            urls="$(pacman -Sp tigervnc fltk1.3 2>/dev/null | sort -u)" || \
                die "Could not resolve TigerVNC packages."
            for url in $urls; do
                wget -q -O "$WORK_DIR/vnc-pkgs/$(basename "$url")" "$url"
            done
            for pkg in "$WORK_DIR/vnc-pkgs"/*.pkg.tar.zst; do
                bsdtar -xf "$pkg" -C "$LOCAL_PREFIX" --strip-components=1
            done
            ;;
        *)
            warn "Install tigervnc-viewer with your package manager to open the screen."
            return
            ;;
    esac
    [ -x "$LOCAL_PREFIX/bin/vncviewer" ] || die "VNC viewer install failed."
    ok "VNC viewer installed: $LOCAL_PREFIX/bin/vncviewer"
}

ensure_vhclient() {
    local src="$LOCAL_PREFIX/bin/vhclient" dst="/usr/local/bin/vhclient"
    if [ ! -x "$src" ]; then
        info "Downloading VirtualHere console client into $LOCAL_PREFIX/bin ..."
        mkdir -p "$LOCAL_PREFIX/bin"
        wget -q -O "$src" \
            https://www.virtualhere.com/sites/default/files/usbclient/vhclientx86_64
        chmod +x "$src"
    fi
    [ -x "$src" ] || die "VirtualHere client download failed."

    # The client needs root to attach devices, and polkit will not run a binary
    # from a home directory, so keep a system-wide copy as well.
    if [ ! -x "$dst" ] || ! cmp -s "$src" "$dst"; then
        info "Installing VirtualHere client to $dst (needs root)..."
        "${ROOT_PREFIX[@]}" install -m 755 "$src" "$dst"
    fi
    ok "VirtualHere client ready: $dst"
}

setup_usb() {
    ensure_vhclient
    info "Loading usbip/vhci kernel modules (may ask for authentication)..."
    "${ROOT_PREFIX[@]}" modprobe usbip-core
    "${ROOT_PREFIX[@]}" modprobe vhci-hcd

    local rule="/etc/udev/rules.d/99-inferno-dfu.rules"
    # DFU (05ac:1227) and Recovery (05ac:1281) are different USB IDs.
    printf '%s\n' \
        'SUBSYSTEM=="usb", ATTR{idVendor}=="05ac", MODE="0666"' \
        | "${ROOT_PREFIX[@]}" tee "$rule" >/dev/null
    "${ROOT_PREFIX[@]}" chmod 644 "$rule"
    "${ROOT_PREFIX[@]}" udevadm control --reload-rules
    "${ROOT_PREFIX[@]}" udevadm trigger --subsystem-match=usb \
        --attr-match=idVendor=05ac 2>/dev/null || true

    if command -v usbmuxd >/dev/null 2>&1; then
        if ! pgrep -x usbmuxd >/dev/null 2>&1; then
            "${ROOT_PREFIX[@]}" systemctl start usbmuxd >/dev/null 2>&1 || \
                (setsid "${ROOT_PREFIX[@]}" usbmuxd -v >/dev/null 2>&1 < /dev/null &)
        fi
        ok "usbmuxd running"
    else
        warn "usbmuxd missing; run ./setup.sh deps first"
    fi
    ok "usbip/vhci + Apple USB udev rule ready"
}

ensure_inferno() {
    [ -f "$INFERNO_DIR/xmake.lua" ] && return
    if [ -d "$SCRIPT_DIR/.git" ]; then
        info "Inferno submodule not initialized; fetching it now..."
        git -C "$SCRIPT_DIR" submodule update --init --recursive Inferno \
            || die "Failed to initialize the Inferno submodule (check network access to git.chefkiss.dev)"
    fi
    [ -f "$INFERNO_DIR/xmake.lua" ] \
        || die "Inferno source not found in $INFERNO_DIR (run: git submodule update --init --recursive)"
}

bootstrap() {
    ensure_inferno
    ensure_xmake
    ensure_lzfse
    xmake --version | head -n 1
}

build_inferno() {
    [ -f "$INFERNO_DIR/xmake.lua" ] || die "Inferno source not found in $INFERNO_DIR"
    info "Building Inferno with xmake (this can take several minutes)..."
    cd "$INFERNO_DIR"
    xmake f -y -m release --qom_cast_debug=n --wxwidgets=n
    xmake -y -j "$JOBS"
    ok "Inferno built (inferno-aarch64, inferno-img, inferno-fs-patcher)"
}

create_disks() {
    mkdir -p "$ROOT_DIR"
    cd "$INFERNO_DIR"
    info "Creating disk images in $ROOT_DIR ..."
    local disk size
    for spec in \
        "root:32G" "firmware:8M" "syscfg:128K" "ctrl_bits:8K" "nvram:8K" \
        "effaceable:4K" "panic_log:1M" "sep_nvram:64K" "sep_ssc:128K"; do
        disk="${spec%%:*}"
        size="${spec##*:}"
        if [ -f "$ROOT_DIR/$disk" ]; then
            info "  $disk already exists, skipping"
            continue
        fi
        xmake r inferno-img create -f raw "$ROOT_DIR/$disk" "$size"
    done
    ok "Disk images ready"
}

fetch_ipsw() {
    local filename="${IPSW_FILENAME:-}"
    local url="${IPSW_URL:-}"
    [ -n "$filename" ] || die "IPSW_FILENAME is not set in config.env"
    [ -n "$url" ] || die "IPSW_URL is not set in config.env"
    mkdir -p "$ROOT_DIR"
    if [ -s "$ROOT_DIR/$filename" ]; then
        ok "IPSW already present: $filename"
        return
    fi
    warn "Downloading IPSW (~5.5 GB): $filename"
    wget -c --show-progress -O "$ROOT_DIR/$filename" "$url"
    ok "IPSW downloaded"
}

prepare_firmware() {
    mkdir -p "$ROOT_DIR"
    local rom="${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1}"
    if [ ! -s "$ROOT_DIR/$rom" ] && [ -n "${SEP_ROM_URL:-}" ]; then
        info "Downloading SEP ROM ..."
        wget --show-progress -O "$ROOT_DIR/$rom" "$SEP_ROM_URL"
    fi
    if [ ! -s "$ROOT_DIR/$rom" ]; then
        warn "SEP ROM missing: $ROOT_DIR/$rom"
        warn "Get the Cebu B1 ROM from https://securerom.fun and put it there,"
        warn "or set SEP_ROM_URL in config.env."
    else
        ok "SEP ROM present: $rom"
    fi

    if [ -z "${SEP_IV_KEY:-}" ]; then
        warn "SEP_IV_KEY is empty. Set it in config.env before running restore."
    else
        ok "SEP IV+Key configured"
    fi
}

build_sss() {
    info "Building SimulatedSigningServer ..."
    if [ ! -f "$SSS_DIR/xmake.lua" ]; then
        git clone --depth 1 https://git.chefkiss.dev/AppleHax/SimulatedSigningServer.git "$SSS_DIR"
    fi
    cd "$SSS_DIR"
    xmake f -y
    xmake -y
    ok "SimulatedSigningServer built"
}

build_restore_tools() {
    info "Building libimobiledevice suite + idevicerestore fork into $LOCAL_PREFIX ..."
    mkdir -p "$WORK_DIR" "$LOCAL_PREFIX"
    local repo
    for repo in libplist libusbmuxd libimobiledevice-glue libtatsu libimobiledevice libirecovery; do
        if [ ! -d "$WORK_DIR/$repo/.git" ]; then
            git clone --depth 1 "https://github.com/libimobiledevice/$repo" "$WORK_DIR/$repo"
        fi
        (
            cd "$WORK_DIR/$repo"
            if [ "$repo" = "libirecovery" ]; then
                # Otherwise make install tries to write /usr/lib/udev/rules.d
                ./autogen.sh --prefix="$LOCAL_PREFIX" \
                    --with-udevrulesdir="$LOCAL_PREFIX/lib/udev/rules.d"
            else
                ./autogen.sh --prefix="$LOCAL_PREFIX"
            fi
            make -j"$JOBS"
            make install
        )
    done
    if [ ! -d "$WORK_DIR/idevicerestore/.git" ]; then
        git clone --depth 1 https://github.com/VisualEhrmanntraut/idevicerestore "$WORK_DIR/idevicerestore"
    fi
    (
        cd "$WORK_DIR/idevicerestore"
        ./autogen.sh --prefix="$LOCAL_PREFIX"
        make -j"$JOBS"
        make install
    )
    ldconfig 2>/dev/null || true
    ok "idevicerestore installed to $LOCAL_PREFIX/bin"
}

check_status() {
    local rc=0
    info "Environment check"
    command -v xmake >/dev/null 2>&1 && ok "xmake: $(command -v xmake)" || { err "xmake missing"; rc=1; }
    [ -f "$LOCAL_PREFIX/include/lzfse.h" ] && ok "lzfse: $LOCAL_PREFIX/include/lzfse.h" || { err "lzfse missing"; rc=1; }
    [ -f "$INFERNO_DIR/xmake.lua" ] && ok "Inferno source: $INFERNO_DIR" || { err "Inferno source missing"; rc=1; }
    [ -f "$SSS_DIR/xmake.lua" ] && ok "SSS source: $SSS_DIR" || { err "SSS source missing"; rc=1; }
    local disk
    for disk in root firmware syscfg ctrl_bits nvram effaceable panic_log sep_nvram sep_ssc; do
        [ -f "$ROOT_DIR/$disk" ] || { err "Missing disk: $disk"; rc=1; }
    done
    [ -s "$ROOT_DIR/${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1}" ] || { err "SEP ROM missing"; rc=1; }
    [ -s "$ROOT_DIR/${IPSW_FILENAME:-}" ] || { warn "IPSW not downloaded yet"; }
    [ -n "${SEP_IV_KEY:-}" ] || { warn "SEP_IV_KEY not set"; }
    command -v idevicerestore >/dev/null 2>&1 && ok "idevicerestore: $(command -v idevicerestore)" || warn "idevicerestore not installed"
    command -v cargo >/dev/null 2>&1 && ok "cargo: $(command -v cargo)" || warn "cargo missing (needed for inferno-vgpu)"
    [ -x "$VGPU_DIR/target/dist-vulkan/bin/inferno-vgpu-server" ] \
        && ok "inferno-vgpu-server: $VGPU_DIR/target/dist-vulkan/bin/inferno-vgpu-server" \
        || warn "inferno-vgpu not built yet (run ./setup.sh gpu)"
    [ -e /dev/udmabuf ] && ok "/dev/udmabuf present" || warn "/dev/udmabuf missing"
    return $rc
}

build_gpu() {
    [ -d "$VGPU_DIR" ] || die "inferno-vgpu directory missing: $VGPU_DIR"
    command -v cargo >/dev/null 2>&1 || die "Rust/cargo is required to build inferno-vgpu"
    info "Building inferno-vgpu (metal2vulkan + server + C library)..."
    (
        cd "$VGPU_DIR"
        cargo xtask dist
        make -C guest/bundle host-test
    )
    ok "inferno-vgpu built ($VGPU_DIR/target/dist-vulkan)"
    build_inferno
}

usage() {
    echo "Usage: $0 [deps|bootstrap|viewer|usb|build|gpu|disks|ipsw|firmware|sss|restore-tools|check|all]"
    echo ""
    echo "  deps          Install system packages via sudo (Arch/Debian)"
    echo "  bootstrap     Install local xmake fallback + build lzfse (no sudo)"
    echo "  viewer        Install a local VNC viewer so you can open the iOS screen"
    echo "  usb           Install VirtualHere client + load usbip/vhci modules (sudo)"
    echo "  build         Configure and build Inferno with xmake"
    echo "  gpu           Build inferno-vgpu (Rust/metal2vulkan) + rebuild Inferno"
    echo "  disks         Create the 9 raw disk images in InfernoData/"
    echo "  ipsw          Download the 5.5 GB IPSW (use SKIP_IPSW=1 with all to skip)"
    echo "  firmware      Verify/download SEP ROM and check IV+Key"
    echo "  sss           Clone and build SimulatedSigningServer"
    echo "  restore-tools Build idevicerestore fork + libimobiledevice locally"
    echo "  check         Report what is ready and what is missing"
    echo "  all           Full setup + start everything + run restore (one command)"
}

main() {
    echo "============================================"
    echo " Inferno Automation Setup (xmake edition)"
    echo "============================================"
    case "${1:-all}" in
        deps)           install_system_deps ;;
        bootstrap)      bootstrap ;;
        viewer)         ensure_vnc_viewer ;;
        usb)            setup_usb ;;
        build)          bootstrap; build_inferno ;;
        gpu)            bootstrap; build_gpu ;;
        disks)          bootstrap; create_disks ;;
        ipsw)           fetch_ipsw ;;
        firmware)       prepare_firmware ;;
        sss)            bootstrap; build_sss ;;
        restore-tools)  build_restore_tools ;;
        check)          check_status ;;
        all)
            install_system_deps
            bootstrap
            ensure_vnc_viewer
            setup_usb
            build_inferno
            create_disks
            build_sss
            prepare_firmware
            [ "${SKIP_IPSW:-0}" = "1" ] || fetch_ipsw
            [ "${SKIP_RESTORE_TOOLS:-0}" = "1" ] || build_restore_tools
            check_status || die "Environment check failed. Fix the items above."
            echo ""
            echo "============================================"
            echo " Setup complete — starting everything now..."
            echo " Logs: $LOG_DIR/sss.log , $LOG_DIR/main.log"
            echo "============================================"
            "$SCRIPT_DIR/run.sh" all
            ;;
        *) usage ;;
    esac
}

main "$@"
