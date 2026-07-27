#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/config.env"

ROOT_DIR="$SCRIPT_DIR/InfernoData"
BUILD_DIR="$SCRIPT_DIR/Inferno/build"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }

check_files() {
    local missing=0
    for f in "$@"; do
        [ ! -f "$ROOT_DIR/$f" ] && { warn "Missing: $f"; missing=1; }
    done
    [ $missing -eq 1 ] && { err "Missing required files. Run setup.sh first."; exit 1; }
}

companion() {
    info "Booting companion VM (USB passthrough mode)..."
    check_files "companion.raw"
    exec "$BUILD_DIR/qemu-system-x86_64" \
        -accel kvm -cpu host \
        -M q35 -m "$COMPANION_RAM" -smp "$COMPANION_SMP" \
        -drive file="$ROOT_DIR/companion.raw",format=raw,if=virtio \
        -usb -device usb-ehci,id=ehci \
        -device usb-tcp-remote,bus=ehci.0 \
        -nic user,model=virtio-net-pci,hostfwd=tcp::"$COMPANION_SSH_PORT"-:22 \
        -serial mon:stdio -nographic
}

main_vm() {
    info "Booting iPhone emulator..."
    check_files "root" "firmware" "syscfg" "ctrl_bits" "nvram" "effaceable" "panic_log" \
                "sep_nvram" "sep_ssc" "root_ticket.der" "sep-firmware.$DEVICE.RELEASE.new.img4" \
                "AppleSEPROM-Cebu-B1"

    exec "$BUILD_DIR/qemu-system-aarch64" \
        -M "$BOARD,trustcache=$ROOT_DIR/Restore/Firmware/$RAM_DISK.trustcache" \
           "ticket=$ROOT_DIR/root_ticket.der" \
           "sep-fw=$ROOT_DIR/sep-firmware.$DEVICE.RELEASE.new.img4" \
           "sep-rom=$ROOT_DIR/$SEP_ROM_NAME" \
           "kaslr-off=true" \
        -kernel "$ROOT_DIR/Restore/kernelcache.research.$DEVICE_FULL" \
        -dtb "$ROOT_DIR/Restore/Firmware/all_flash/DeviceTree.$DEVICE.im4p" \
        -append "tlto_us=-1 mtxspin=-1 agm-genuine=1 agm-authentic=1 agm-trusted=1 serial=3 wdt=-1 -vm_compressor_wk_sw" \
        -smp "$MAIN_SMP" -m "$MAIN_RAM" \
        -display gtk,zoom-to-fit=on,show-cursor=on \
        -serial mon:stdio \
        -initrd "$ROOT_DIR/Restore/$RAM_DISK" \
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

restore_cmd() {
    echo ""
    echo "Run this command INSIDE the companion VM (after booting main VM):"
    echo ""
    echo "  idevicerestore --erase --restore-mode \\"
    echo "    -i 0x1122334455667788 \\"
    echo "    /root/$IPSW_FILENAME \\"
    echo "    -T /root/root_ticket.der"
    echo ""
}

usage() {
    echo "Usage: $0 [companion|main|restore]"
    echo ""
    echo "  companion   Boot companion VM (with USB passthrough)"
    echo "  main        Boot iPhone emulator (main VM)"
    echo "  restore     Show restore command for companion"
}

case "${1:-help}" in
    companion) companion ;;
    main) main_vm ;;
    restore) restore_cmd ;;
    *) usage ;;
esac
