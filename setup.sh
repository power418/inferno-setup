#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/config.env"

ROOT_DIR="$SCRIPT_DIR/InfernoData"
INFERNO_DIR="$SCRIPT_DIR/Inferno"
BUILD_DIR="$INFERNO_DIR/build"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
err()   { echo -e "${RED}[ERR]${NC} $1"; }

detect_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$ID"
    else
        echo "unknown"
    fi
}

install_deps() {
    info "Installing dependencies..."
    local distro=$(detect_distro)
    case "$distro" in
        arch|artix|manjaro|cachyos|endeavouros)
            pacman -Sy --noconfirm git glib2 dtc pixman zlib libtasn1 meson ninja \
                base-devel cmake gnutls pkgconf sdl2 libssh gtk3 lzfse \
                python python-pip python-pyasn1 python-pyasn1-modules 2>/dev/null
            # Install nettle >= 3.10 if needed
            if [ "$(pacman -Qi nettle 2>/dev/null | awk '/Version/ {print $3}' | cut -d. -f1)" -lt 3 ]; then
                warn "Nettle too old, building from source..."
                cd /tmp
                wget -q https://ftpmirror.gnu.org/gnu/nettle/nettle-3.10.2.tar.gz
                tar -xf nettle-3.10.2.tar.gz && cd nettle-3.10.2
                ./configure && make -j$(nproc) && make install
            fi
            ;;
        debian|ubuntu|pop)
            apt-get update -y
            apt-get install -y build-essential libtool meson ninja-build pkg-config \
                device-tree-compiler libglib2.0-dev gnutls-bin libjpeg-turbo8-dev \
                libpng-dev libslirp-dev libssh-dev libusb-1.0-0-dev liblzo2-dev \
                libncurses5-dev libpixman-1-dev libsnappy-dev vde2 zstd libzstd-dev \
                libgnutls28-dev libgmp10 libgmp3-dev lzfse liblzfse-dev libgtk-3-dev \
                libsdl2-dev python3 python3-pip python3-pyasn1 python3-pyasn1-modules
            # Build nettle if too old
            if [ "$(apt-cache show nettle 2>/dev/null | awk '/Version/ {print $2}' | cut -d. -f1)" -lt 3 ]; then
                warn "Nettle too old, building from source..."
                cd /tmp
                wget -q https://ftpmirror.gnu.org/gnu/nettle/nettle-3.10.2.tar.gz
                tar -xf nettle-3.10.2.tar.gz && cd nettle-3.10.2
                ./configure && make -j$(nproc) && make install
            fi
            ;;
        *)
            err "Unsupported distro. Install dependencies manually."
            exit 1
            ;;
    esac
    ok "Dependencies installed"
}

build_inferno() {
    info "Cloning and building Inferno..."
    if [ ! -d "$INFERNO_DIR" ]; then
        git clone https://github.com/ChefKissInc/Inferno "$INFERNO_DIR"
        cd "$INFERNO_DIR"
        git submodule update --init
    else
        cd "$INFERNO_DIR"
    fi
    mkdir -p "$BUILD_DIR" && cd "$BUILD_DIR"
    ../configure --target-list=aarch64-softmmu,x86_64-softmmu \
        --enable-lzfse --enable-slirp --enable-curses --enable-libssh \
        --enable-virtfs --enable-zstd --enable-nettle --enable-gnutls \
        --enable-gtk --enable-sdl --disable-werror --disable-qom-cast-debug \
        --disable-debug-info
    make -j$(nproc)
    ok "Inferno built"
}

create_disks() {
    info "Creating disk images..."
    mkdir -p "$ROOT_DIR" && cd "$ROOT_DIR"
    for disk in root firmware syscfg ctrl_bits nvram effaceable panic_log sep_nvram sep_ssc; do
        [ -f "$disk" ] && continue
        case "$disk" in
            root) "$BUILD_DIR/qemu-img" create -f raw root 32G ;;
            firmware) "$BUILD_DIR/qemu-img" create -f raw firmware 8M ;;
            syscfg) "$BUILD_DIR/qemu-img" create -f raw syscfg 128K ;;
            ctrl_bits) "$BUILD_DIR/qemu-img" create -f raw ctrl_bits 8K ;;
            nvram) "$BUILD_DIR/qemu-img" create -f raw nvram 8K ;;
            effaceable) "$BUILD_DIR/qemu-img" create -f raw effaceable 4K ;;
            panic_log) "$BUILD_DIR/qemu-img" create -f raw panic_log 1M ;;
            sep_nvram) "$BUILD_DIR/qemu-img" create -f raw sep_nvram 64K ;;
            sep_ssc) "$BUILD_DIR/qemu-img" create -f raw sep_ssc 128K ;;
        esac
    done
    ok "Disks created"
}

fetch_ipsw() {
    info "Downloading IPSW (this may take a while)..."
    cd "$ROOT_DIR"
    if [ ! -f "$IPSW_FILENAME" ]; then
        wget -q --show-progress "$IPSW_URL"
    fi
    if [ ! -d "Restore" ]; then
        mkdir Restore
        unzip -q "$IPSW_FILENAME" -d Restore
    fi
    # Remove the largest DMG to save space
    rm -f Restore/*.dmg
    ok "IPSW extracted"
}

create_apticket() {
    info "Creating AP ticket..."
    cd "$ROOT_DIR"
    # Download scripts
    wget -q https://chefkiss.dev/Extras/Inferno/create_apticket.py -O create_apticket.py
    wget -q "$TICKET_SHSH_URL" -O ticket.shsh2
    run_with_venv python3 create_apticket.py "$DEVICE" Restore/BuildManifest.plist ticket.shsh2 root_ticket.der
    ok "AP ticket created"
}

prepare_sep() {
    info "Preparing SEP firmware..."
    cd "$ROOT_DIR"
    # Check if SEP ROM exists
    if [ ! -f "$SEP_ROM_NAME" ] && [ -n "$SEP_ROM_URL" ]; then
        wget -q "$SEP_ROM_URL" -O "$SEP_ROM_NAME"
    fi
    # Build img4lib
    if [ ! -d "img4lib" ]; then
        git clone --depth 1 https://github.com/xerub/img4lib
        cd img4lib && make && cd "$ROOT_DIR"
    fi
    # Create SEP ticket
    wget -q https://chefkiss.dev/Extras/Inferno/create_septicket.py -O create_septicket.py
    run_with_venv python3 create_septicket.py "$DEVICE" Restore/BuildManifest.plist ticket.shsh2 sep_root_ticket.der
    # Decrypt SEP firmware
    if [ -n "$SEP_IV_KEY" ]; then
        cd "$ROOT_DIR"
        SEP_VERSION=$(./img4lib/img4 -v -i Restore/Firmware/all_flash/sep-firmware.$DEVICE.RELEASE.im4p -o sep-firmware.$DEVICE.RELEASE -k $SEP_IV_KEY 2>/dev/null || ./img4lib/img4 -v -i Restore/Firmware/all_flash/sep-firmware.n104.RELEASE.im4p -o sep-firmware.n104.RELEASE -k $SEP_IV_KEY)
        ./img4lib/img4 -A -F -o sep-firmware.$DEVICE.RELEASE.new.img4 \
            -i sep-firmware.$DEVICE.RELEASE \
            -M sep_root_ticket.der -T rsep -V $SEP_VERSION
        ok "SEP firmware prepared"
    else
        warn "SEP_IV_KEY not set. Decrypt manually:"
        warn "  cd $ROOT_DIR"
        warn "  img4lib/img4 -v -i Restore/Firmware/all_flash/sep-firmware.$DEVICE.RELEASE.im4p -o sep-firmware.$DEVICE.RELEASE -k IVKEY"
        warn "  img4lib/img4 -A -F -o sep-firmware.$DEVICE.RELEASE.new.img4 -i sep-firmware.$DEVICE.RELEASE -M sep_root_ticket.der -T rsep -V VERSION"
    fi
}

setup_companion() {
    info "Setting up companion VM..."
    cd "$ROOT_DIR"
    # Download Arch ISO
    if [ ! -f "archlinux.iso" ]; then
        wget -q --show-progress -O archlinux.iso \
            "https://geo.mirror.pkgbuild.com/iso/latest/archlinux-x86_64.iso"
    fi
    # Create companion disk
    if [ ! -f "companion.raw" ]; then
        "$BUILD_DIR/qemu-img" create -f raw companion.raw 20G
    fi
    ok "Companion disk ready"
    warn "Boot the companion VM manually and install Arch Linux:"
    warn "  $BUILD_DIR/qemu-system-x86_64 -accel kvm -cpu host -M q35 -m $COMPANION_RAM -smp $COMPANION_SMP"
    warn "    -drive file=companion.raw,format=raw,if=virtio -cdrom archlinux.iso"
    warn "    -nic user,model=virtio-net-pci -serial mon:stdio"
    warn ""
    warn "After Arch is installed in companion, run this on the host to build tools:"
    warn "  cd $ROOT_DIR"
    warn "  $BUILD_DIR/qemu-system-x86_64 -accel kvm -cpu host -M q35 -m $COMPANION_RAM -smp $COMPANION_SMP"
    warn "    -drive file=companion.raw,format=raw,if=virtio"
    warn "    -nic user,model=virtio-net-pci,hostfwd=tcp::$COMPANION_SSH_PORT-:22"
    warn "    -serial mon:stdio -nographic"
    warn ""
    warn "Then from host, run: ./setup.sh --build-companion-tools"
}

build_companion_tools() {
    info "Building libimobiledevice tools in companion via SSH..."
    local SSH_CMD="sshpass -p '$COMPANION_PASS' ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p $COMPANION_SSH_PORT $COMPANION_USER@localhost"
    # Install deps
    $SSH_CMD "pacman -Syu --noconfirm base-devel git glib2 openssl cmake libtool automake autoconf pkg-config libusb libzip libpng"
    # Clone and build all repos
    $SSH_CMD 'cd /root
        for repo in libplist libusbmuxd libimobiledevice-glue libtatsu libimobiledevice libirecovery usbmuxd idevicerestore; do
            git clone --depth 1 https://github.com/libimobiledevice/$repo
            cd $repo
            ./autogen.sh && make -j$(nproc) && make install
            cd /root
        done
        # Patch idevicerestore
        cd /root/idevicerestore
        curl -sL https://chefkiss.dev/Extras/Inferno/idevicerestore.patch -o idevicerestore.patch
        git apply idevicerestore.patch
        ./autogen.sh && make -j$(nproc) && make install
        ldconfig'
    # Transfer IPSW and ticket
    sshpass -p "$COMPANION_PASS" scp -P $COMPANION_SSH_PORT "$ROOT_DIR/$IPSW_FILENAME" $COMPANION_USER@localhost:/root/
    sshpass -p "$COMPANION_PASS" scp -P $COMPANION_SSH_PORT "$ROOT_DIR/root_ticket.der" $COMPANION_USER@localhost:/root/
    ok "Companion tools built and files transferred"
}

setup_python() {
    info "Setting up Python virtual environment for ticket scripts..."
    cd "$ROOT_DIR"
    if [ ! -d "venv" ]; then
        python3 -m venv venv
    fi
    source venv/bin/activate
    pip3 install pyasn1 pyasn1-modules 2>/dev/null
    ok "Python venv ready"
}

run_with_venv() {
    cd "$ROOT_DIR"
    source venv/bin/activate
    "$@"
}

main() {
    echo "============================================"
    echo " Inferno Automation Setup"
    echo "============================================"

    case "${1:-all}" in
        deps) install_deps ;;
        python) setup_python ;;
        build) build_inferno ;;
        disks) create_disks ;;
        ipsw) fetch_ipsw ;;
        ticket) create_apticket ;;
        sep) prepare_sep ;;
        companion) setup_companion ;;
        companion-tools) build_companion_tools ;;
        all)
            install_deps
            setup_python
            build_inferno
            create_disks
            fetch_ipsw
            create_apticket
            prepare_sep
            echo ""
            echo "============================================"
            echo " Setup complete!"
            echo " Next: run './run.sh companion' to boot companion VM"
            echo "       run './run.sh main' to boot iPhone emulator"
            echo "============================================"
            ;;
        *)
            echo "Usage: $0 [deps|python|build|disks|ipsw|ticket|sep|companion|companion-tools|all]"
            ;;
    esac
}

main "$@"
