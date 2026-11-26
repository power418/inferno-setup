SDKROOT ?= $(shell xcrun --sdk iphoneos --show-sdk-path 2>/dev/null)
KDK_INC ?= $(SDKROOT)/System/Library/Frameworks/Kernel.framework/Headers
WIRE_INC = ../../crates/inferno-vgpu-graphics/include

.PHONY: all clean
all:
ifndef SDKROOT
	$(error SDKROOT is not set; build on macOS with iOS/Kernel SDK headers)
endif
	@mkdir -p build/InfernoVgpu.kext/Contents/MacOS
	cp Info.plist build/InfernoVgpu.kext/Contents/Info.plist
	clang++ -arch arm64e -isysroot $(SDKROOT) -mkernel -nostdinc++ -fno-rtti -fno-exceptions \
		-I$(KDK_INC) -I$(WIRE_INC) -c src/InfernoVgpuDriver.cpp -o build/InfernoVgpuDriver.o
	clang++ -arch arm64e -isysroot $(SDKROOT) -mkernel -nostdlib -lkmod \
		build/InfernoVgpuDriver.o -o build/InfernoVgpu.kext/Contents/MacOS/InfernoVgpu

clean:
	rm -rf build
