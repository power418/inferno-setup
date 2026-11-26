# Inferno vGPU Firmware / Prelink Package (`IVGF`) Build & Verification
#
# Targets:
#   make overlay       Stage an overlay-only InfernoVgpu.fw (safe without iOS SDK)
#   make verify        Verify staged build/InfernoVgpu.fw
#   make header-check  Syntax-check include/inferno_firmware_abi.h with clang -std=c11
#   make kc-inspect KC=<path/to/kernelcache> [LZFSE_DIR=...]
#   make clean         Remove build/

FW_TOOL   ?= ../target/release/inferno-vgpu-fw
BUILD     ?= build
LZFSE_DIR ?= ../../.local/lib
KC        ?=

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

verify: $(FW_TOOL) $(BUILD)/InfernoVgpu.fw
	$(FW_TOOL) verify $(BUILD)/InfernoVgpu.fw

kc-inspect: $(FW_TOOL)
	@test -n "$(KC)" || { echo "Usage: make kc-inspect KC=/path/to/kernelcache.release.iphone12b"; exit 2; }
	$(FW_TOOL) inspect-kc "$(KC)" --lzfse-dir "$(LZFSE_DIR)"

clean:
	rm -rf $(BUILD)

.PHONY: all header-check overlay verify kc-inspect clean
