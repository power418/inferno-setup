ifeq ($(origin CXX),default)
CXX = clang++
endif
IOS_ARCH ?= arm64
IOS_MIN ?= 14.0
SDKROOT ?= $(shell xcrun --sdk iphoneos --show-sdk-path 2>/dev/null)

WIRE_INC = ../../crates/inferno-vgpu-graphics/include

HOST_CXXFLAGS = -std=c++17 -O2 -Wall -Wextra -Werror -pthread -I$(WIRE_INC) -Isrc

.PHONY: all ios host-test clean

all: host-test

host-test: build/wire_abi_test
	./build/wire_abi_test

build/wire_abi_test: src/InfernoWireClient.cpp src/InfernoWireClient.hpp tests/wire_abi_test.cpp $(WIRE_INC)/inferno_graphics_wire.h
	@mkdir -p build
	$(CXX) $(HOST_CXXFLAGS) src/InfernoWireClient.cpp tests/wire_abi_test.cpp -o $@

ios:
ifndef SDKROOT
	$(error SDKROOT is not set. Run on macOS with Xcode or pass SDKROOT=/path/to/iPhoneOS.sdk)
endif
	@mkdir -p build/ios
	$(CXX) -arch $(IOS_ARCH) -isysroot $(SDKROOT) -miphoneos-version-min=$(IOS_MIN) \
		-std=c++17 -ObjC++ -fobjc-arc -O2 -dynamiclib \
		-install_name /usr/lib/libInfernoMetal.dylib \
		-I$(WIRE_INC) -Isrc \
		-framework Foundation -framework Metal -framework IOSurface -framework IOKit -framework QuartzCore \
		src/InfernoWireClient.cpp src/InfernoMetal.mm \
		-o build/ios/libInfernoMetal.dylib

clean:
	rm -rf build
