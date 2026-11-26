# Top-level Guest Orchestrator for inferno-vgpu (iOS arm64e)
# Mirrors reims-vgpu/guest/Makefile layout.
#
# Targets:
#   make host-test   Compile & run Linux host C++17 wire ABI & ring tests
#   make sdk-check   Check macOS Xcode / Linux iPhoneOS.sdk cross-compile prerequisites
#   make manifest    Generate guest/build/sources.toml SHA-256 record
#   make kc-check KC=<kernelcache> KEXT=<kext-macho>
#   make clean       Remove guest build outputs

FW_TOOL   ?= ../target/release/inferno-vgpu-fw
LZFSE_DIR ?= ../../.local/lib
KC        ?=
KEXT      ?=

all: host-test manifest

host-test:
	$(MAKE) -C bundle host-test

sdk-check:
	bash tools/ios-cross-check.sh

manifest:
	python3 -I -B tools/bundle_manifest.py record build/sources.toml

manifest-check:
	python3 -I -B tools/bundle_manifest.py check build/sources.toml

kc-check:
	@test -n "$(KC)" && test -n "$(KEXT)" || { echo "Usage: make kc-check KC=<kernelcache> KEXT=<kext-macho>"; exit 2; }
	$(FW_TOOL) check-imports "$(KC)" "$(KEXT)" --lzfse-dir "$(LZFSE_DIR)"

clean:
	$(MAKE) -C bundle clean
	rm -rf build

.PHONY: all host-test sdk-check manifest manifest-check kc-check clean
