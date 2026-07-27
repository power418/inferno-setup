# Inferno Automation

> Automated setup scripts for [Inferno](https://github.com/ChefKissInc/Inferno) — iPhone 11 emulator for Linux.

## Prerequisites

| Item | Description |
|------|-------------|
| **CPU** | x86_64 with KVM support (Intel VT-x / AMD-V) |
| **RAM** | 8 GB minimum (4 GB for main VM + 2 GB for companion) |
| **Storage** | 45 GB free space |
| **OS** | Linux (Arch-based or Debian-based) |

Supported devices:
- iPhone 11 (`iPhone12,1` / `n104ap`)
- iPhone XR (`iPhone11,8` / `n841ap`)

---

## Quick Start

### 1. Clone & Configure

```bash
git clone https://github.com/YOUR_USER/inferno-automation.git
cd inferno-automation
```

Edit `config.env`:
- Set `IPSW_URL` to your IPSW download link
- Set `SEP_IV_KEY` (get from [The Apple Wiki](https://theapplewiki.com))
- Set `SEP_ROM_URL` (get from [securerom.fun](https://securerom.fun))

### 2. Run Full Setup

```bash
./setup.sh all
```

This will:
- Install dependencies (auto-detects Arch/Debian)
- Clone & build Inferno (QEMU fork)
- Create all required disk images
- Download & extract IPSW
- Create AP ticket
- Prepare SEP firmware (if `SEP_IV_KEY` is set)

### 3. Setup Companion VM

```bash
# Boot companion installer
./setup.sh companion
# Follow interactive Arch Linux installation
# After installation, boot companion with SSH:
./run.sh companion
# In another terminal, build tools inside companion:
./setup.sh companion-tools
```

### 4. Boot & Restore

**Terminal 1** — Boot companion with USB passthrough:
```bash
./run.sh companion
```

**Terminal 2** — Boot iPhone emulator:
```bash
./run.sh main
```

**Terminal 1 (in companion)** — Run restore:
```bash
idevicerestore --erase --restore-mode \
  -i 0x1122334455667788 \
  /root/iPhone11,8,iPhone12,1_14.0_18A5351d_Restore.ipsw \
  -T /root/root_ticket.der
```

---

## Step-by-Step Guide

### Host Setup

Dependencies by distribution:

| Distribution | Command |
|-------------|---------|
| **Arch Linux** | `sudo pacman -S git glib2 dtc pixman zlib libtasn1 meson ninja base-devel cmake gnutls pkgconf sdl2 libssh gtk3 lzfse` |
| **Debian/Ubuntu** | `sudo apt-get install -y build-essential libtool meson ninja-build pkg-config device-tree-compiler libglib2.0-dev gnutls-bin libjpeg-turbo8-dev libpng-dev libslirp-dev libssh-dev libusb-1.0-0-dev liblzo2-dev libncurses5-dev libpixman-1-dev libsnappy-dev vde2 zstd libzstd-dev libgnutls28-dev libgmp10 libgmp3-dev lzfse liblzfse-dev libgtk-3-dev libsdl2-dev` |

> **Note:** Some distros ship Nettle < 3.10. The script builds it from source automatically.

### Python Setup (for Ticket Scripts)

Install via system packages:

| Distribution | Command |
|-------------|---------|
| **Arch Linux** | `sudo pacman -S python python-pip python-pyasn1 python-pyasn1-modules` |
| **Debian/Ubuntu** | `sudo apt-get install -y python3 python3-pip python3-pyasn1 python3-pyasn1-modules` |

Or use a virtual environment (recommended if packages aren't available):

```bash
python3 -m venv venv
source venv/bin/activate
pip3 install pyasn1 pyasn1-modules
```

The `setup.sh` script handles this automatically with `./setup.sh python`.

### Building Inferno

```bash
git clone https://github.com/ChefKissInc/Inferno
cd Inferno
git submodule update --init
mkdir build && cd build
../configure --target-list=aarch64-softmmu,x86_64-softmmu \
  --enable-lzfse --enable-slirp --enable-curses --enable-libssh \
  --enable-virtfs --enable-zstd --enable-nettle --enable-gnutls \
  --enable-gtk --enable-sdl --disable-werror \
  --disable-qom-cast-debug --disable-debug-info
make -j$(nproc)
```

### File Setup

#### Disk Images

| File | Size | Purpose |
|------|------|---------|
| `root` | 32 GB | Main OS data partition |
| `firmware` | 8 MB | Firmware storage |
| `syscfg` | 128 KB | System configuration |
| `ctrl_bits` | 8 KB | Control bits |
| `nvram` | 8 KB | NVRAM storage |
| `effaceable` | 4 KB | Effaceable storage |
| `panic_log` | 1 MB | Panic log storage |
| `sep_nvram` | 64 KB | SEP NVRAM |
| `sep_ssc` | 128 KB | SEP SSC |

#### IPSW Extraction

```bash
mkdir Restore
unzip iPhone11,8,iPhone12,1_14.0_18A5351d_Restore.ipsw -d Restore
# Delete largest DMG to save space (~5 GB)
rm Restore/*.dmg
```

#### Creating AP Ticket

```bash
# Activate venv first (if using venv)
source venv/bin/activate

# Download scripts
wget https://chefkiss.dev/Extras/Inferno/create_apticket.py
wget https://chefkiss.dev/Extras/Inferno/ticket.shsh2

# Generate ticket
python3 create_apticket.py n104ap Restore/BuildManifest.plist ticket.shsh2 root_ticket.der
```

> **Important:** Do not modify the generated ticket. It's required for all boot stages.

#### SEP Firmware Preparation

1. Get the SEP ROM from [securerom.fun](https://securerom.fun) (must be `Cebu B1` for t8030)

2. Build img4lib:
```bash
git clone https://github.com/xerub/img4lib
cd img4lib && make
```

3. Create SEP ticket:
```bash
wget https://chefkiss.dev/Extras/Inferno/create_septicket.py
python3 create_septicket.py n104ap Restore/BuildManifest.plist ticket.shsh2 sep_root_ticket.der
```

4. Decrypt & repackage SEP firmware (get IV+Key from [The Apple Wiki](https://theapplewiki.com)):
```bash
img4lib/img4 -v -i Restore/Firmware/all_flash/sep-firmware.n104.RELEASE.im4p \
  -o sep-firmware.n104.RELEASE -k IVKEY
img4lib/img4 -A -F -o sep-firmware.n104.RELEASE.new.img4 \
  -i sep-firmware.n104.RELEASE \
  -M sep_root_ticket.der -T rsep -V VERSION
```

> Replace `IVKEY` with IV and Key concatenated (no spaces). Replace `VERSION` with output from first command.

### Companion VM Setup

The companion VM provides USB connectivity to the emulated iPhone. It runs a lightweight Linux distro with `libimobiledevice` tools.

#### Arch Linux Installation

```bash
qemu-system-x86_64 -accel kvm -cpu host -M q35 -m 2G -smp 2 \
  -drive file=companion.raw,format=raw,if=virtio \
  -cdrom archlinux.iso \
  -nic user,model=virtio-net-pci \
  -serial mon:stdio
```

Follow standard Arch installation inside the VM:
- Partition: single ext4 root partition
- Install `base base-devel linux linux-firmware grub dhcpcd openssh sshpass`
- Set root password
- Enable `sshd` and `dhcpcd`
- Install GRUB: `grub-install /dev/sda && grub-mkconfig -o /boot/grub/grub.cfg`

#### Building libimobiledevice Tools

Build order (each with `./autogen.sh && make -j$(nproc) && make install`):

| Repo | Description |
|------|-------------|
| [libplist](https://github.com/libimobiledevice/libplist) | Apple property list parser/generator |
| [libusbmuxd](https://github.com/libimobiledevice/libusbmuxd) | USB multiplexing library |
| [libimobiledevice-glue](https://github.com/libimobiledevice/libimobiledevice-glue) | Common library |
| [libtatsu](https://github.com/libimobiledevice/libtatsu) | Apple authentication library |
| [libimobiledevice](https://github.com/libimobiledevice/libimobiledevice) | Main library |
| [libirecovery](https://github.com/libimobiledevice/libirecovery) | iDevice recovery library |
| [usbmuxd](https://github.com/libimobiledevice/usbmuxd) | USB multiplexer daemon |
| [idevicerestore](https://github.com/libimobiledevice/idevicerestore) | Restore tool |

#### idevicerestore Patch

Apply the [Inferno patch](https://chefkiss.dev/Extras/Inferno/idevicerestore.patch):

```bash
cd idevicerestore
curl -LO https://chefkiss.dev/Extras/Inferno/idevicerestore.patch
git apply idevicerestore.patch
./autogen.sh && make -j$(nproc) && make install
ldconfig
```

---

## Running

### Companion VM (with USB passthrough)

```bash
./run.sh companion
```

This starts the companion with the `usb-tcp-remote` device that connects to the main VM.

### Main VM (iPhone Emulator)

```bash
./run.sh main
```

Boot command breakdown:

| Flag | Description |
|------|-------------|
| `-M t8030` | Machine type (iPhone 11 SoC) |
| `trustcache=...` | Trust cache for kernel |
| `ticket=root_ticket.der` | AP ticket for iBoot |
| `sep-fw=...` | SEP firmware |
| `sep-rom=...` | SEP ROM (Cebu B1) |
| `kaslr-off=true` | Disable KASLR for debugging |
| `-kernel` | Research kernel cache |
| `-dtb` | Device tree blob |
| `-append` | Kernel boot arguments |
| `-initrd` | RAM disk (Erase or Upgrade) |
| `-smp 7 -m 4G` | 7 cores, 4 GB RAM |

> **Tip:** For GUI display issues, add `-display gtk,zoom-to-fit=on,show-cursor=on` (Linux) or `-display cocoa,zoom-to-fit=on,zoom-interpolation=on,show-cursor=on` (macOS).

### Running Restore

In the companion VM (after main VM has booted and shows USB connected):

```bash
idevicerestore --erase --restore-mode \
  -i 0x1122334455667788 \
  /root/iPhone11,8,iPhone12,1_14.0_18A5351d_Restore.ipsw \
  -T /root/root_ticket.der
```

For restore stage 2 (data migration), apply [filesystem patches](https://chefkiss.dev/guides/inferno/fs-patches/) in the iOS VM.

---

## Troubleshooting

### Kernel Panic: "Attempted to kill init!"

Add `-cpu host` to QEMU command. Required for AVX CPU instructions.

### `at24c-eeprom: Backing file size 65536 != 2048`

Fix SEP NVRAM size:
```bash
truncate -c -s 64K sep_nvram
```

### Stuck near "skipping unencrypted volume" or "SEP accepted IMG4"

Restart the emulator. May take multiple attempts.

### Home/App Switcher gesture broken

Trigger SOS/power-off screen, then either:
1. Tap X to close, double-press Home → App Switcher → swipe Setup away
2. Slide to power off, restart emulator

### IDE Device Not Found

Ensure companion starts BEFORE main VM. The main VM connects to the companion, not the other way around.

### USB Connection Issues

- Use UNIX sockets (default) on Linux
- Only one USB controller on main VM
- No XHCI controller (unsupported)
- Check `lsusb` in companion

---

## Reference

| Component | Source |
|-----------|--------|
| Inferno QEMU fork | [github.com/ChefKissInc/Inferno](https://github.com/ChefKissInc/Inferno) |
| Official guide | [chefkiss.dev/guides/inferno](https://chefkiss.dev/guides/inferno/) |
| IPSW downloads | [ipsw.me](https://ipsw.me) / [The Apple Wiki](https://theapplewiki.com) |
| SEP ROM dumps | [securerom.fun](https://securerom.fun) |
| IV/Keys | [The Apple Wiki](https://theapplewiki.com) |
| libimobiledevice | [github.com/libimobiledevice](https://github.com/libimobiledevice) |
| Support | [Telegram](https://t.me/+Bx3MO9Hq8whhNzk9) |

---

## License

Scripts are provided as-is for educational purposes. Respect Apple's EULA — do not share IPSW files, decrypted firmware, or modification tools.
