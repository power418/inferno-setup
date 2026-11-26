ifeq ($(origin CXX),default)
CXX = clang++
endif
IOS_ARCH ?= arm64e
IOS_MIN ?= 14.0
SDKROOT ?= $(shell xcrun --sdk iphoneos --show-sdk-path 2>/dev/null)
STUBS_ROOT ?= ../build/sdk-stubs
FW_TOOL ?= ../../target/release/inferno-vgpu-fw

WIRE_INC = ../../crates/inferno-vgpu-graphics/include

HOST_CXXFLAGS = -std=c++17 -O2 -Wall -Wextra -Werror -pthread -I$(WIRE_INC) -Isrc

.PHONY: all ios linux-arm64e host-test clean

all: host-test

host-test: build/wire_abi_test
	./build/wire_abi_test

build/wire_abi_test: src/InfernoWireClient.cpp src/InfernoWireClient.hpp tests/wire_abi_test.cpp $(WIRE_INC)/inferno_graphics_wire.h
	@mkdir -p build
	$(CXX) $(HOST_CXXFLAGS) src/InfernoWireClient.cpp tests/wire_abi_test.cpp -o $@

linux-arm64e:
	@python3 -I -B ../tools/extract_ios_stubs.py "$(STUBS_ROOT)" >/dev/null
	@mkdir -p build/ios/InfernoMetal.bundle
	cp Info.plist build/ios/InfernoMetal.bundle/Info.plist
	$(CXX) -target $(IOS_ARCH)-apple-ios$(IOS_MIN) -std=c++17 -O2 -fPIC -fno-exceptions \
		-fno-rtti -fno-stack-protector -fno-threadsafe-statics -nostdinc++ \
		-DINFERNO_BUNDLE_FREESTANDING -Ifreestanding -I$(WIRE_INC) -Isrc \
		-c src/InfernoWireClient.cpp -o build/ios/InfernoWireClient.o
	$(CXX) -target $(IOS_ARCH)-apple-ios$(IOS_MIN) -std=c++17 -O2 -fPIC -fno-exceptions \
		-fno-rtti -fno-stack-protector -fno-threadsafe-statics -nostdinc++ \
		-DINFERNO_BUNDLE_FREESTANDING -Ifreestanding -I$(WIRE_INC) -Isrc \
		-c src/InfernoMetalBundleEntry.cpp -o build/ios/InfernoMetalBundleEntry.o
	ld64.lld -arch $(IOS_ARCH) -platform_version ios $(IOS_MIN) $(IOS_MIN) -dylib \
		-install_name /System/Library/Extensions/InfernoMetal.bundle/InfernoMetal \
		build/ios/InfernoWireClient.o build/ios/InfernoMetalBundleEntry.o \
		$(STUBS_ROOT)/usr/lib/libSystem.B.tbd \
		$(STUBS_ROOT)/System/Library/Frameworks/IOKit.framework/IOKit.tbd \
		-o build/ios/InfernoMetal.bundle/InfernoMetal.unsigned
	$(FW_TOOL) adhoc-sign build/ios/InfernoMetal.bundle/InfernoMetal.unsigned \
		--out build/ios/InfernoMetal.bundle/InfernoMetal \
		--id dev.chefkiss.metal.InfernoMetal > build/ios/InfernoMetal.cdhash
	rm -f build/ios/InfernoMetal.bundle/InfernoMetal.unsigned

ios:
ifndef SDKROOT
	$(error SDKROOT is not set. Run on macOS with Xcode or use 'make linux-arm64e' with .tbd stubs)
endif
	@mkdir -p build/ios
	$(CXX) -arch $(IOS_ARCH) -isysroot $(SDKROOT) -miphoneos-version-min=$(IOS_MIN) \
		-std=c++17 -ObjC++ -fobjc-arc -O2 -dynamiclib \
		-install_name /System/Library/Extensions/InfernoMetal.bundle/InfernoMetal \
		-I$(WIRE_INC) -Isrc \
		-framework Foundation -framework Metal -framework IOSurface -framework IOKit -framework QuartzCore \
		src/InfernoWireClient.cpp src/InfernoMetal.mm \
		-o build/ios/libInfernoMetal.dylib

clean:
	rm -rf build
