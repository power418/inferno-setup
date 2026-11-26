# Inferno - iPhone QEMU Virtualization Manager

Automation scripts for [Inferno](https://github.com/ChefKissInc/Inferno) — an iPhone 11 / XR (t8030) emulator built from a heavily modified QEMU.

> These scripts target the **current** Inferno checkout, which uses **xmake** (`inferno-aarch64`, `inferno-img`). The old `configure && make` / companion-VM flow does not work with this source tree.

## Prerequisites

| Item | Description |
|------|-------------|
| CPU | x86_64, ideally AVX2 (12th-gen Intel here). No system QEMU needed — Inferno is built from source. |
| RAM | 8 GB minimum (4 GB for the emulated iPhone) |
| Storage | ~45 GB free (IPSW alone is ~5.5 GB) |
| OS | Arch-based (CachyOS tested) or Debian/Ubuntu |
| Tools | `git`, `curl`/`wget`, `cmake`, `python3`, `sudo` for system packages |

Supported devices: iPhone 11 (`iPhone12,1` / `n104ap`) and iPhone XR (`iPhone11,8` / `n841ap`) with iOS 14.0 18A5351d by default.

## macOS (`.command` launchers)

The original `setup.sh` / `run.sh` are Linux-oriented (pacman/apt, udev, usbip/vhci, systemd). For macOS use the double-clickable launchers in Finder, or run them from Terminal:

```bash
./accel-check.command   # host + accelerator verdict (do this first)
./setup-mac.command     # MacPorts deps, clone, build, disks, firmware, IPSW
./run-mac.command       # start signing server + emulator in background
./run-mac.command main  # emulator in foreground; pass any subcommand explicitly
./run-mac.command view  # Screen Sharing → VNC 127.0.0.1:5900
./run-mac.command stop
```

All logic is in `inferno-mac.command`; the other `.command` files are thin launchers. It installs dependencies with **MacPorts** (`sudo port install`, no Homebrew/udev/kernel modules), builds lzfse locally into `.local/`, initializes the tracked `Inferno` submodule (or clones it from `git.chefkiss.dev/AppleHax/Inferno` when no submodule is configured), and always passes an explicit `-accel` to the emulator. Set `MACPORTS_PREFIX` if your MacPorts prefix is not `/opt/local`; `PORT_SELFUPDATE=1` runs `sudo port selfupdate` first.

**Acceleration reality check:** Inferno only compiles `hvf` on **arm64 macOS** and `kvm` on **arm64 Linux** (`xmake/options.lua`). On an Intel Mac, `sysctl kern.hv_support=1` only means Hypervisor.framework can run **x86_64** guests — it cannot accelerate the ARM64 iPhone guest. So on Intel macOS the only accelerator is **TCG** (software JIT). On native Apple Silicon, `ACCEL=auto` selects HVF; override with `ACCEL=tcg` if needed. `./accel-check.command` prints the verdict and, once built, smoke-tests the binary with `-accel <name> -S`.

USB attach/restore on macOS still needs the VirtualHere client and `idevicerestore` manually; the macOS launcher only brings up the signing server and emulator.

## What gets installed

- System dependencies via `pacman`/`apt` (glib2, nettle, libtasn1, lz4, gnutls, libpng, libyuv, zstd, …).
- `xmake` (system package, with a `~/.local` fallback installer).
- [lzfse](https://github.com/lzfse/lzfse) built locally into `.local/` (xmake requires it for DMG support).
- [SimulatedSigningServer](https://git.chefkiss.dev/AppleHax/SimulatedSigningServer.git) — the fake Tatsu signing server.
- Optional: patched [idevicerestore](https://github.com/VisualEhrmanntraut/idevicerestore) + libimobiledevice suite.
- VirtualHere console client + `usbip`/`vhci-hcd` kernel modules, so the emulated iPhone appears as a real USB device to `idevicerestore`.
- `usbmuxd` (started by `./run.sh usb`) so `idevicerestore` can talk to the device once it enters Restore mode.

## Configuration

Edit `config.env`:

- `IPSW_URL` / `IPSW_FILENAME` — any supported IPSW (default URL still works).
- `SEP_ROM_NAME` — keep `AppleSEPROM-Cebu-B1`; place the ROM in `InfernoData/` or set `SEP_ROM_URL` (get it from [securerom.fun](https://securerom.fun)).
- `SEP_IV_KEY` — IV+Key of the IPSW's SEP firmware, concatenated with no spaces ([The Apple Wiki](https://theapplewiki.com)). Needed for restore.
- `MAIN_SMP`, `MAIN_RAM`, `DISPLAY_ARGS` — tune if needed.

## Setup

Inferno is tracked as a git submodule, so clone recursively (or initialize it later):

```bash
git clone --recurse-submodules https://github.com/power418/inferno-setup.git
cd inferno-setup
# already cloned without it? run: git submodule update --init --recursive
```

Then:

```bash
./setup.sh deps          # installs system packages (sudo password prompt)
./setup.sh all           # bootstrap + build + disks + SSS + firmware + IPSW + tools
```

To skip the 5.5 GB download or the libimobiledevice build:

```bash
SKIP_IPSW=1 SKIP_RESTORE_TOOLS=1 ./setup.sh all
```

Individual steps are also available: `bootstrap`, `viewer`, `usb`, `build`, `disks`, `ipsw`, `firmware`, `sss`, `restore-tools`, `check`. `viewer` installs a local TigerVNC viewer (no sudo) so `./run.sh view` can open the screen. `usb` loads `usbip-core`/`vhci-hcd`, installs the VirtualHere client to `/usr/local/bin` (it needs root; polkit refuses binaries under `$HOME`), and adds an Apple USB udev rule so `idevicerestore` can access both DFU (`05ac:1227`) and Recovery (`05ac:1281`) modes.

What `all` produces:

| Path | Purpose |
|------|---------|
| `Inferno/` | Inferno source + xmake build output |
| `InfernoData/root` etc. | 9 raw disk images (32 GB root + firmware/nvram/…) |
| `InfernoData/<IPSW>.ipsw` | Restore IPSW |
| `InfernoData/AppleSEPROM-Cebu-B1` | SEP ROM |
| `SimulatedSigningServer/` | SSS source + build |
| `.local/` | lzfse, optional idevicerestore |

## Run

One command does setup, starts both services, attaches USB, and runs restore:

```bash
cd ~/Desktop/inferno-setup
./setup.sh all
```

Already set up (e.g. after a reboot)? Skip straight to the same one-shot run:

```bash
./run.sh all
```

Logs: `/tmp/opencode/sss.log` and `/tmp/opencode/main.log`.

Set `SKIP_RESTORE=1` to bring up the services and attach USB without running restore (useful for debugging):
```bash
SKIP_RESTORE=1 ./run.sh all
```

To view the screen while it runs, open another terminal:
```bash
./run.sh view
```

`./run.sh view` opens `vncview.py`, a small GTK3 viewer that scales the 832×1792 iPhone screen to fit your monitor (aspect ratio kept, window resizable). Mouse click/drag = touch. It needs `python-gobject` + GTK3. Use `VIEWER=tiger ./run.sh view` to fall back to TigerVNC (no scaling).

Manual/three-terminal mode is still available:

```bash
# 1. Fake signing server
./run.sh sss

# 2. Emulated iPhone (VirtualHere USB uplink defaults to 127.0.0.1:8030)
./run.sh main

# 2b. Attach the emulated iPhone over VirtualHere (auto-called by restore too)
./run.sh usb

# 3. Print or run the restore command
./run.sh restore      # print it
./run.sh restore-now  # execute it directly
```

`./run.sh restore` only prints the command; `./run.sh restore-now` (used by `all`) executes it:

```bash
idevicerestore --erase -i 0x1122334455667788 \
  InfernoData/iPhone11,8,iPhone12,1_14.0_18A5351d_Restore.ipsw \
  -s http://127.0.0.1:8031/ \
  --sep-key "IVKEY" --dev-as-ap
```

### Required: filesystem patches after restore

After a successful restore you **must** apply the filesystem patches before booting normally. They include the software-rendering patches for the dyld shared cache; without them iOS boots (launchd, keybag, etc. in the log) but the screen stays **completely black**.

Stop the emulator first (the root image must not be in use), then:

```bash
./run.sh stop                    # graceful SIGTERM for emulator + USB client + signing server
./run.sh fs-patch                # ends with "Changes applied successfully."
./run.sh all                     # boot again and re-attach the iPhone over USB
./run.sh view                    # in another terminal
```

`./run.sh stop` (alias: `./run.sh kill`) stops the emulator, the SimulatedSigningServer and the VirtualHere client. It uses SIGTERM first, so the SEP state stays consistent; SIGKILL is only a fallback for processes that refuse to exit. After `stop`, `./run.sh all` (or `./run.sh main` followed by `./run.sh usb`) brings everything back up, including USB passthrough.

`inferno-fs-patcher --revert` undoes the patches; `--dry-run` shows them without writing.

## Useful details

- Current Inferno has no GTK/SDL display on Linux; the official guide runs headless with `-serial mon:stdio`. `DISPLAY_ARGS` is empty by default. The emulator still starts a VNC server on `localhost:5900`, which you can open with any VNC client.
- `-M t8030,...kaslr-off=true` mirrors the official guide; `usb-uplink-addr` is left at the built-in default (`127.0.0.1:8030`).
- `./run.sh usb` also turns on VirtualHere **Auto-Use All**. This matters: after iBSS is uploaded the iPhone disconnects and reconnects in Recovery Mode, and auto-use is what attaches the new USB device automatically.
- No AP ticket, `img4lib`, or companion VM is used by this Inferno version.
- The old README's `ticket.shsh2`, `create_apticket.py`, and `sep_root_ticket.der` steps are obsolete here.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `liblzfse is required` during xmake configure | `./setup.sh bootstrap`, then re-run `./setup.sh build` |
| `nettle >= 3.10` error | `./setup.sh deps` (script builds 3.10 locally on older distros) |
| Emulator shows no USB device | `./setup.sh usb` then `./run.sh usb`; check `lsusb -d 05ac:` and `sudo -n /usr/local/bin/vhclient -t LIST` |
| `Unable to discover device mode` | Check `lsusb -d 05ac:`; if the node is `crw-rw-r--`, re-run `./setup.sh usb` (Apple udev rule) |
| `Device did not reconnect ... Possibly invalid iBSS` | Make sure VirtualHere `Auto-Use All` is on (`./run.sh usb`), then retry |
| Screen is black after restore, log shows normal launchd boot | Filesystem patches missing: stop the emulator, `./run.sh fs-patch`, boot again |
| `SEP Panic: :sars/sars` / `wdog panic` early in boot | SEP state got inconsistent after a hard kill; use `./run.sh stop` (SIGTERM) next time, then boot again |
| VNC window taller than the monitor | Use `./run.sh view` (scaled viewer) instead of TigerVNC |
| Restore fails signing | Confirm `./run.sh sss` is listening on `SSS_PORT` (8031) |
| `idevicerestore: command not found` | `./setup.sh restore-tools` |
| SEP errors | Check the ROM is Cebu B1 and `SEP_IV_KEY` matches the IPSW |

## Reference

- Inferno source: <https://github.com/ChefKissInc/Inferno>
- Official guide: <https://chefkiss.dev/guides/inferno/>
- SEP ROM dump used here (A13 / Cebu B1): <https://securerom.fun/resources/SEPROM/AppleSEPROM-Cebu-B1>
- IV/Key for iOS 14.0 beta 5 (18A5351d, iPhone 11): <https://theapplewiki.com/wiki/Keys:AzulSeed_18A5351d_(iPhone12,1)> — already filled into `config.env` as `SEP_IV_KEY`.

Respect Apple's EULA — do not share IPSWs, decrypted firmware, or keys.
