# Inferno vGPU Firmware / Prelink Package (`IVGF`) Build & Verification
#
# Targets:
#   make overlay       Stage an overlay-only InfernoVgpu.fw (safe without guest binaries)
#   make full          Stage 5-entry InfernoVgpu.fw (dt-overlay, arm64e kext, signed Metal bundle, CDHash, optional reloc-map)
#   make verify        Verify staged build/InfernoVgpu.fw
#   make header-check  Syntax-check include/inferno_firmware_abi.h with clang -std=c11
#   make kc-inspect KC=<path/to/kernelcache> [LZFSE_DIR=...]
#   make clean         Remove build/

FW_TOOL   ?= ../target/release/inferno-vgpu-fw
BUILD     ?= build
LZFSE_DIR ?= ../../.local/lib
KC        ?=
KEXT_BIN  ?= ../guest/driver/build/InfernoVgpu.kext/Contents/MacOS/InfernoVgpu
KEXT_PLIST?= ../guest/driver/Info.plist
BUNDLE_BIN?= ../guest/bundle/build/ios/InfernoMetal.bundle/InfernoMetal
CDHASH    ?= ../guest/bundle/build/ios/InfernoMetal.cdhash
RELOC_BIN ?= ../guest/build/abi/reloc-map.bin

all: header-check overlay verify

$(FW_TOOL):
	cd .. && cargo build --release -p inferno-vgpu-firmware

header-check:
	clang -x c-header -std=c11 -fsyntax-only -Wall -Wextra -Werror include/inferno_firmware_abi.h

overlay: $(FW_TOOL)
	@mkdir -p $(BUILD)
	$(FW_TOOL) stage \
		--out $(BUILD)/InfernoVgpu.fw \
		--featureset GLES2,0 \
		--dt-overlay dev.inferno.pvgpu.dt-overlay:dt-overlay.plist

full: header-check $(FW_TOOL)
	@mkdir -p $(BUILD)
	@test -f "$(KEXT_BIN)" && test -f "$(BUNDLE_BIN)" && test -f "$(CDHASH)" || { \
		echo "Building guest arm64e artifacts..."; \
		$(MAKE) -C ../guest linux-arm64e; \
	}
	$(FW_TOOL) stage \
		--out $(BUILD)/InfernoVgpu.fw \
		--featureset GLES2,0 \
		--dt-overlay dev.inferno.pvgpu.dt-overlay:dt-overlay.plist \
		--kext dev.chefkiss.driver.InfernoVgpu:$(KEXT_BIN):$(KEXT_PLIST) \
		--bundle dev.chefkiss.metal.InfernoMetal:$(BUNDLE_BIN):/System/Library/Extensions/InfernoMetal.bundle/InfernoMetal \
		--cdhash dev.chefkiss.metal.InfernoMetal:$(CDHASH) \
		$(if $(wildcard $(RELOC_BIN)),--reloc-map dev.inferno.pvgpu.reloc-map:$(RELOC_BIN))
	$(FW_TOOL) verify $(BUILD)/InfernoVgpu.fw

verify: $(FW_TOOL) $(BUILD)/InfernoVgpu.fw
	$(FW_TOOL) verify $(BUILD)/InfernoVgpu.fw

kc-inspect: $(FW_TOOL)
	@test -n "$(KC)" || { echo "Usage: make kc-inspect KC=/path/to/kernelcache.release.iphone12b"; exit 2; }
	$(FW_TOOL) inspect-kc "$(KC)" --lzfse-dir "$(LZFSE_DIR)"

clean:
	rm -rf $(BUILD)

.PHONY: all header-check overlay full verify kc-inspect clean
