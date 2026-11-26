SDKROOT ?= $(shell xcrun --sdk iphoneos --show-sdk-path 2>/dev/null)

.PHONY: all clean
all:
ifndef SDKROOT
	$(error SDKROOT is not set; build on macOS with Xcode + Metal toolchain)
endif
	@mkdir -p build
	xcrun -sdk iphoneos metal -c shaders.metal -o build/shaders.air
	xcrun -sdk iphoneos metallib build/shaders.air -o build/shaders.metallib
	clang++ -arch arm64 -isysroot $(SDKROOT) -miphoneos-version-min=14.0 -std=c++17 -ObjC++ -fobjc-arc \
		-framework Foundation -framework Metal main.mm -o build/inferno-gpu-test

clean:
	rm -rf build
