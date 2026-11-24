#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/config.env"

ROOT_DIR="$SCRIPT_DIR/InfernoData"
INFERNO_DIR="$SCRIPT_DIR/Inferno"
SSS_DIR="$SCRIPT_DIR/SimulatedSigningServer"
LOCAL_PREFIX="${LOCAL_PREFIX:-$SCRIPT_DIR/.local}"

export PATH="$LOCAL_PREFIX/bin:$HOME/.local/bin:$PATH"
export LD_LIBRARY_PATH="$LOCAL_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

ROOT_PREFIX=()
if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
    ROOT_PREFIX=(sudo -n)
fi

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()  { echo -e "${CYAN}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
err()   { echo -e "${RED}[ERR]${NC} $1" >&2; }
die()   { err "$1"; exit 1; }

LOG_DIR="${TMPDIR:-/tmp}/opencode"
SSS_PORT="${SSS_PORT:-8031}"
USB_PORT="8030"
RESTORE_MARKER="$ROOT_DIR/.restored"

require_built() {
    [ -f "$INFERNO_DIR/xmake.lua" ] || { err "Inferno source missing. Run ./setup.sh build"; exit 1; }
    command -v xmake >/dev/null 2>&1 || { err "xmake missing. Run ./setup.sh bootstrap"; exit 1; }
}

check_files() {
    local missing=0 f
    for f in "$@"; do
        [ -s "$ROOT_DIR/$f" ] || { warn "Missing/empty: $f"; missing=1; }
    done
    if [ "$missing" -eq 1 ]; then
        err "Required files are missing. Run ./setup.sh disks and ./setup.sh firmware first."
        exit 1
    fi
}

main_vm() {
    require_built
    info "Booting iPhone emulator (t8030)..."
    check_files "root" "firmware" "syscfg" "ctrl_bits" "nvram" "effaceable" \
                "panic_log" "sep_nvram" "sep_ssc" "${SEP_ROM_NAME:-AppleSEPROM-Cebu-B1}"

    local display_args=()
    [ -n "${DISPLAY_ARGS:-}" ] && display_args=(-display "$DISPLAY_ARGS")

    cd "$INFERNO_DIR"
    exec xmake r inferno-aarch64 \
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

signing_server() {
    if [ ! -f "$SSS_DIR/xmake.lua" ]; then
        err "SimulatedSigningServer missing. Run ./setup.sh sss"
        exit 1
    fi
    info "Starting SimulatedSigningServer on ${SSS_HOST:-127.0.0.1}:${SSS_PORT:-8031} ..."
    cd "$SSS_DIR"
    exec xmake r SimulatedSigningServer "${SSS_HOST:-127.0.0.1}" "${SSS_PORT:-8031}"
}

vh_list() {
    "${ROOT_PREFIX[@]}" "$1" -t LIST 2>/dev/null || true
}

start_vhclient() {
    pgrep -x vhclient >/dev/null 2>&1 && return 0
    info "Starting VirtualHere client daemon (root)..."
    setsid "${ROOT_PREFIX[@]}" "$1" -n >/dev/null 2>&1 < /dev/null &
    sleep 2
}

restart_vhclient() {
    if pgrep -x vhclient >/dev/null 2>&1; then
        "${ROOT_PREFIX[@]}" pkill -x vhclient >/dev/null 2>&1 || \
            pkill -x vhclient >/dev/null 2>&1 || true
        sleep 1
    fi
    start_vhclient "$1"
}

attach_usb() {
    local vh="/usr/local/bin/vhclient" i listing addr
    [ -x "$vh" ] || vh="$LOCAL_PREFIX/bin/vhclient"
    if [ ! -x "$vh" ]; then
        err "VirtualHere client missing. Run ./setup.sh usb first."
        return 1
    fi

    # Self-heal: kernel modules do not survive a reboot.
    if ! vhci_loaded; then
        info "vhci_hcd missing — running ./setup.sh usb ..."
        "$SCRIPT_DIR/setup.sh" usb >/dev/null 2>&1 || true
        for i in $(seq 1 20); do
            vhci_loaded && break
            sleep 0.5
        done
        if ! vhci_loaded; then
            err "Could not load vhci_hcd. Run ./setup.sh usb manually."
            return 1
        fi
    fi

    if command -v usbmuxd >/dev/null 2>&1 && ! pgrep -x usbmuxd >/dev/null 2>&1; then
        info "Starting usbmuxd..."
        "${ROOT_PREFIX[@]}" systemctl start usbmuxd >/dev/null 2>&1 || \
            (setsid "${ROOT_PREFIX[@]}" usbmuxd -v >/dev/null 2>&1 < /dev/null &)
        sleep 1
    fi

    start_vhclient "$vh"

    # The VirtualHere hub lives inside the emulator. If the emulator died
    # (e.g. an early SEP panic), restart it before blaming the USB client.
    local attempt
    for attempt in $(seq 1 15); do
        if ! port_open "$USB_PORT"; then
            warn "Emulator is not listening on $USB_PORT — booting it again..."
            start_vm_bg || return 1
        fi

        "${ROOT_PREFIX[@]}" "$vh" -t "MANUAL HUB ADD,127.0.0.1:8030" >/dev/null 2>&1 || true
        sleep 2
        listing="$(vh_list "$vh")"

        # Auto-use re-attaches the iPhone after iBSS makes it reconnect.
        if printf '%s\n' "$listing" | grep -q 'Auto-Use All currently off'; then
            "${ROOT_PREFIX[@]}" "$vh" -t "AUTO USE ALL" >/dev/null 2>&1 || true
            sleep 1
            listing="$(vh_list "$vh")"
        fi

        addr="$(printf '%s\n' "$listing" \
            | grep -o '([^()]*\.[0-9][0-9]*)' \
            | tail -n 1 | tr -d '()' || true)"
        [ -n "$addr" ] && break

        if [ $((attempt % 3)) -eq 0 ]; then
            info "Refreshing VirtualHere connection (attempt $attempt/15)..."
            restart_vhclient "$vh"
        fi
        sleep 1
    done

    if [ -z "$addr" ]; then
        err "iPhone not visible through VirtualHere yet."
        printf '%s\n' "$listing"
        return 1
    fi
    info "Attaching $addr ..."
    "${ROOT_PREFIX[@]}" "$vh" -t "USE,$addr" >/dev/null 2>&1 || true
    listing="$(vh_list "$vh")"
    printf '%s\n' "$listing" | grep -- '-->' || true
    lsusb -d 05ac: 2>/dev/null || true
}

RESTORE_BIN=""
RESTORE_ARGS=()

restore_prepare() {
    attach_usb || warn "USB attachment failed; restore will likely not see the iPhone."
    local ipsw="$ROOT_DIR/${IPSW_FILENAME:-}"
    local key="${SEP_IV_KEY:-YOUR_IPSW_SEP_KEY_HERE}"
    RESTORE_BIN="idevicerestore"
    [ -x "$LOCAL_PREFIX/bin/idevicerestore" ] && RESTORE_BIN="$LOCAL_PREFIX/bin/idevicerestore"
    [ -s "$ipsw" ] || die "IPSW not found: $ipsw (run ./setup.sh ipsw)"
    [ -n "${SEP_IV_KEY:-}" ] || die "SEP_IV_KEY is empty (set it in config.env)"

    RESTORE_ARGS=(
        --erase
        -i "${ECID:-0x1122334455667788}"
        "$ipsw"
        -s "http://${SSS_HOST:-127.0.0.1}:${SSS_PORT}/"
        --sep-key "$key"
        --dev-as-ap
    )
}

restore_cmd() {
    restore_prepare
    echo ""
    echo "Ready to run (services are up if you used ./run.sh all):"
    echo ""
    printf '  "%s"' "$RESTORE_BIN"
    printf ' %q' "${RESTORE_ARGS[@]}"
    printf '\n\n'
}

restore_now() {
    restore_prepare
    info "Running restore (this can take a while)..."
    if "$RESTORE_BIN" "${RESTORE_ARGS[@]}"; then
        : > "$RESTORE_MARKER"
        ok "Restore finished (marker: $RESTORE_MARKER)"
    else
        local rc=$?
        err "Restore failed (exit $rc)"
        return "$rc"
    fi
}

fs_patch() {
    require_built
    info "Applying filesystem patches to root image..."
    cd "$INFERNO_DIR"
    exec xmake r inferno-fs-patcher -d "$ROOT_DIR/root"
}

view_screen() {
    local viewer
    if [ "${VIEWER:-fit}" != "tiger" ] && \
       python3 -c 'import gi; gi.require_version("Gtk","3.0"); from gi.repository import Gtk' 2>/dev/null; then
        info "Opening iOS screen (scaled to fit) at 127.0.0.1:5900 ..."
        exec python3 "$SCRIPT_DIR/vncview.py" 127.0.0.1 5900
    fi
    if command -v vncviewer >/dev/null 2>&1; then
        viewer="$(command -v vncviewer)"
    elif [ -x "$LOCAL_PREFIX/bin/vncviewer" ]; then
        viewer="$LOCAL_PREFIX/bin/vncviewer"
    else
        err "VNC viewer missing. Run ./setup.sh viewer first."
        exit 1
    fi
    info "Opening iOS screen at 127.0.0.1:5900 ..."
    exec env "LD_LIBRARY_PATH=$LOCAL_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
        "$viewer" 127.0.0.1:5900
}

vhci_loaded() {
    # Avoid `lsmod | grep -q` under pipefail: grep -q closes the pipe early
    # and makes lsmod report SIGPIPE, which falsely looks like "not loaded".
    [[ "$(lsmod 2>/dev/null)" == *vhci_hcd* ]]
}

port_open() {
    [[ "$(ss -ltn 2>/dev/null)" == *":${1} "* ]]
}

# PIDs holding a listening socket (ss needs root to show foreign-owned pids,
# but the emulator and signing server run as our user).
port_pids() {
    local port="$1"
    ss -ltnp 2>/dev/null | grep ":${port} " | grep -o 'pid=[0-9]*' \
        | cut -d= -f2 | sort -u || true
}

signal_pids() {
    local sig="$1" pids="$2" root="$3"
    [ -n "$pids" ] || return 0
    if [ "$root" = "1" ]; then
        # vhclient runs as root. Try sudo -n first (works when running as root
        # or with a passwordless sudoers rule), then a non-interactive polkit
        # authorization, then a plain kill for the root shell case.
        if ! "${ROOT_PREFIX[@]}" kill "-$sig" $pids 2>/dev/null; then
            kill "-$sig" $pids 2>/dev/null || \
            { command -v pkexec >/dev/null 2>&1 && \
              timeout 10 pkexec --disable-internal-agent kill "-$sig" $pids \
                  >/dev/null 2>&1; } || true
        fi
    else
        kill "-$sig" $pids 2>/dev/null || true
    fi
}

alive_pids() {
    local pids="$1" p alive=""
    for p in $pids; do
        ps -p "$p" >/dev/null 2>&1 && alive="$alive $p"
    done
    printf '%s' "${alive# }"
}

# Prefer SIGTERM: a SIGKILL'd emulator leaves SEP state inconsistent and the
# next boot panics (which also takes the VirtualHere USB uplink down).
terminate_pids() {
    local pids="$1" label="$2" root="${3:-0}" i alive
    [ -n "$pids" ] || return 0
    info "Stopping $label (PID: ${pids// /, })..."
    signal_pids TERM "$pids" "$root"
    for i in $(seq 1 40); do
        alive="$(alive_pids "$pids")"
        [ -z "$alive" ] && { ok "$label stopped"; return 0; }
        pids="$alive"
        sleep 0.25
    done
    warn "$label ignored SIGTERM — sending SIGKILL"
    signal_pids KILL "$pids" "$root"
    sleep 0.5
    if [ -n "$(alive_pids "$pids")" ]; then
        warn "$label is still running (root-owned processes need sudo/pkexec)"
        return 1
    fi
    return 0
}

wait_port() {
    local port="$1" i
    for i in $(seq 1 60); do
        port_open "$port" && return 0
        sleep 0.5
    done
    return 1
}

start_sss_bg() {
    if port_open "$SSS_PORT"; then
        ok "Signing server already listening on $SSS_PORT"
        return
    fi
    info "Starting signing server on $SSS_PORT ..."
    mkdir -p "$LOG_DIR"
    setsid "$SCRIPT_DIR/run.sh" sss >"$LOG_DIR/sss.log" 2>&1 < /dev/null &
    wait_port "$SSS_PORT" || die "Signing server failed to start (see $LOG_DIR/sss.log)"
}

start_vm_bg() {
    local attempt
    for attempt in 1 2 3; do
        if port_open "$USB_PORT" && pgrep -x inferno-aarch64 >/dev/null 2>&1; then
            ok "Emulator already listening on $USB_PORT"
            return 0
        fi
        if [ "$attempt" -gt 1 ]; then
            warn "Emulator stopped during early boot — retrying ($attempt/3)..."
        else
            info "Booting emulator (log: $LOG_DIR/main.log) ..."
        fi
        mkdir -p "$LOG_DIR"
        setsid "$SCRIPT_DIR/run.sh" main >"$LOG_DIR/main.log" 2>&1 < /dev/null &
        # An inconsistent SEP state can panic ~10-15s into boot. Wait long
        # enough to catch it, otherwise attach_usb would blame VirtualHere.
        if wait_port "$USB_PORT" && sleep 15 && pgrep -x inferno-aarch64 >/dev/null 2>&1; then
            ok "Emulator is up on $USB_PORT"
            return 0
        fi
        warn "Emulator did not stay up (see $LOG_DIR/main.log)"
        sleep 2
    done
    die "Emulator failed to stay up after 3 attempts (see $LOG_DIR/main.log)"
}

ensure_usb_support() {
    if vhci_loaded; then
        return 0
    fi
    info "Preparing USB support (./setup.sh usb)..."
    "$SCRIPT_DIR/setup.sh" usb >/dev/null 2>&1 || true
}

stop_vm() {
    local pids
    pids="$({ port_pids "$USB_PORT"; pgrep -x inferno-aarch64; \
              pgrep -f 'xmake r inferno-aarch64'; } 2>/dev/null \
            | sort -u | paste -sd' ' -)"
    terminate_pids "$pids" "emulator"
}

stop_signing() {
    local pids
    pids="$({ port_pids "$SSS_PORT"; pgrep -x SimulatedSigningServer; \
              pgrep -f 'xmake r SimulatedSigningServer'; } 2>/dev/null \
            | sort -u | paste -sd' ' -)"
    terminate_pids "$pids" "signing server"
}

stop_vhclient() {
    local pids
    pids="$(pgrep -x vhclient 2>/dev/null | paste -sd' ' - || true)"
    [ -n "$pids" ] || return 0
    if ! terminate_pids "$pids" "VirtualHere client" 1; then
        warn "Run: sudo $0 stop"
        return 1
    fi
}

stop_all() {
    info "Stopping Inferno services..."
    local rc=0
    stop_vhclient || rc=1
    stop_vm || rc=1
    stop_signing || rc=1

    # Sockets can linger briefly after the process is gone.
    local port i
    for port in "$USB_PORT" "$SSS_PORT"; do
        for i in $(seq 1 20); do
            port_open "$port" || break
            sleep 0.25
        done
        if port_open "$port"; then
            warn "Something is still listening on $port"
            rc=1
        fi
    done

    if [ "$rc" -eq 0 ]; then
        ok "All stopped. Start again with: ./run.sh all"
    else
        warn "Some services could not be stopped; see the warnings above."
    fi
    return "$rc"
}

run_all() {
    require_built
    info "Starting everything in one shot..."
    ensure_usb_support
    start_sss_bg

    # Never kill a healthy emulator here: an abrupt kill leaves the SEP in an
    # inconsistent state and the next boot panics, taking USB down with it.
    if [ "${FORCE_RESTORE:-0}" = "1" ]; then
        warn "FORCE_RESTORE=1 — restarting emulator for a fresh restore"
        rm -f "$RESTORE_MARKER"
        stop_vm
    fi
    start_vm_bg

    if [ "${SKIP_RESTORE:-0}" = "1" ]; then
        ok "SKIP_RESTORE=1 — services are up, restore skipped"
        return
    fi
    if [ -f "$RESTORE_MARKER" ] && [ "${FORCE_RESTORE:-0}" != "1" ]; then
        ok "Restore already completed — bringing the iPhone online"
        attach_usb || return 1
        info "Open the screen with: ./run.sh view  (VNC 127.0.0.1:5900)"
        return
    fi
    restore_now
    info "Open the screen with: ./run.sh view  (VNC 127.0.0.1:5900)"
}

usage() {
    echo "Usage: $0 [all|stop|sss|main|usb|view|restore|restore-now|fs-patch]"
    echo ""
    echo "  all         Start signing server + emulator + USB; restore only if needed"
    echo "  stop        Gracefully stop emulator, signing server and VirtualHere (USB)"
    echo "  sss         Start SimulatedSigningServer (terminal 1)"
    echo "  main        Boot the emulated iPhone (terminal 2)"
    echo "  usb         Attach the emulated iPhone via VirtualHere client"
    echo "  view        Open the iOS screen with VNC viewer (127.0.0.1:5900)"
    echo "  restore     Print the idevicerestore command"
    echo "  restore-now Run the idevicerestore command directly"
    echo "  fs-patch    Run inferno-fs-patcher on InfernoData/root"
    echo ""
    echo "Env: SKIP_RESTORE=1 (services only), FORCE_RESTORE=1 (restore again)."
    echo "Legacy 'companion' mode is no longer used by current Inferno builds."
}

case "${1:-help}" in
    all|auto)           run_all ;;
    stop|kill)          stop_all ;;
    sss|signing-server) signing_server ;;
    main)               main_vm ;;
    usb)                attach_usb ;;
    view)               view_screen ;;
    restore)            restore_cmd ;;
    restore-now)        restore_now ;;
    fs-patch)           fs_patch ;;
    companion)          warn "Companion VM is obsolete in current Inferno; use './run.sh all'."; usage ;;
    *)                  usage ;;
esac
