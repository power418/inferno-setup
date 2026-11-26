# Top-level Guest Orchestrator for inferno-vgpu (iOS arm64e)
# Mirrors reims-vgpu/guest/Makefile layout.
#
# Targets:
#   make host-test     Compile & run Linux host C++17 wire ABI & ring tests
#   make stubs         Generate iOS 14.0 arm64/arm64e .tbd linker stubs
#   make linux-arm64e  Cross-compile InfernoVgpu.kext + signed InfernoMetal.bundle on Linux
#   make sdk-check     Check macOS Xcode / Linux iPhoneOS.sdk cross-compile prerequisites
#   make manifest      Generate guest/build/sources.toml SHA-256 record
#   make manifest-check Verify guest/build/sources.toml matches current sources
#   make kc-check KC=<kernelcache> [KEXT=driver/build/InfernoVgpu.kext/Contents/MacOS/InfernoVgpu]
#   make clean         Remove guest build outputs

FW_TOOL   ?= ../target/release/inferno-vgpu-fw
LZFSE_DIR ?= ../../.local/lib
KC        ?=
KEXT      ?= driver/build/InfernoVgpu.kext/Contents/MacOS/InfernoVgpu

all: host-test linux-arm64e manifest

host-test:
	$(MAKE) -C bundle host-test

stubs:
	python3 -I -B tools/extract_ios_stubs.py build/sdk-stubs

linux-arm64e: stubs
	$(MAKE) -C driver linux-arm64e
	$(MAKE) -C bundle linux-arm64e

sdk-check:
	bash tools/ios-cross-check.sh

manifest:
	python3 -I -B tools/bundle_manifest.py record build/sources.toml

manifest-check:
	python3 -I -B tools/bundle_manifest.py check build/sources.toml

kc-check:
	@test -n "$(KC)" && test -f "$(KEXT)" || { echo "Usage: make kc-check KC=<kernelcache> [KEXT=...]"; exit 2; }
	$(FW_TOOL) check-imports "$(KC)" "$(KEXT)" --lzfse-dir "$(LZFSE_DIR)"

clean:
	$(MAKE) -C bundle clean
	$(MAKE) -C driver clean
	rm -rf build/sdk-stubs

.PHONY: all host-test stubs linux-arm64e sdk-check manifest manifest-check kc-check clean
