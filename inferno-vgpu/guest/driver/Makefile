SDKROOT ?= $(shell xcrun --sdk iphoneos --show-sdk-path 2>/dev/null)
KDK_INC ?= $(SDKROOT)/System/Library/Frameworks/Kernel.framework/Headers
WIRE_INC = ../../crates/inferno-vgpu-graphics/include
IOS_ARCH ?= arm64e

KEXT_OUT = build/InfernoVgpu.kext/Contents/MacOS/InfernoVgpu

.PHONY: all linux-arm64e clean

all:
ifndef SDKROOT
	$(error SDKROOT is not set; use 'make linux-arm64e' on Linux or build on macOS with Xcode SDK)
endif
	@mkdir -p build/InfernoVgpu.kext/Contents/MacOS
	cp Info.plist build/InfernoVgpu.kext/Contents/Info.plist
	clang++ -arch $(IOS_ARCH) -isysroot $(SDKROOT) -mkernel -nostdinc++ -fno-rtti -fno-exceptions \
		-I$(KDK_INC) -I$(WIRE_INC) -c src/InfernoVgpuDriver.cpp -o build/InfernoVgpuDriver.o
	clang++ -arch $(IOS_ARCH) -isysroot $(SDKROOT) -mkernel -nostdlib -lkmod \
		build/InfernoVgpuDriver.o -o $(KEXT_OUT)

# Linux cross-build produces a relocatable arm64e Mach-O (MH_OBJECT) because
# ld64.lld's `-r` relocatable KEXT link mode is not implemented yet.
linux-arm64e:
	@mkdir -p build/InfernoVgpu.kext/Contents/MacOS
	cp Info.plist build/InfernoVgpu.kext/Contents/Info.plist
	clang++ -target $(IOS_ARCH)-apple-ios14.0 -DINFERNO_FREESTANDING_IOS14 \
		-mkernel -ffreestanding -nostdinc++ -fno-rtti -fno-exceptions \
		-fno-threadsafe-statics -fno-stack-protector -O2 -Wall -Wextra -Werror \
		-Iinclude -I$(WIRE_INC) -c src/InfernoVgpuDriver.cpp -o $(KEXT_OUT)

clean:
	rm -rf build
