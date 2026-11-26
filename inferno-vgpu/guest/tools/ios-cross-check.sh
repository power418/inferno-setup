#!/usr/bin/env bash
set -euo pipefail

echo "=== inferno-vgpu iOS arm64e Guest Toolchain Check ==="
HOST_OS="$(uname -s)"
echo "Host OS: $HOST_OS"

for tool in clang clang++ ld64.lld python3; do
    if command -v "$tool" >/dev/null 2>&1; then
        echo "[OK] $tool: $(command -v "$tool")"
    else
        echo "[MISSING] $tool"
    fi
done

if command -v ldid >/dev/null 2>&1; then
    echo "[OK] ldid: $(command -v ldid)"
else
    echo "[WARN] ldid not found (needed on Linux to ad-hoc sign iOS Mach-O bundles)"
fi

SDK_PATH="${IOS_SDK_PATH:-}"
if [ -z "$SDK_PATH" ] && [ "$HOST_OS" = "Darwin" ] && command -v xcrun >/dev/null 2>&1; then
    SDK_PATH="$(xcrun --sdk iphoneos --show-sdk-path 2>/dev/null || true)"
fi

if [ -n "$SDK_PATH" ] && [ -d "$SDK_PATH/System/Library/Frameworks/Metal.framework" ]; then
    echo "[OK] iPhoneOS SDK: $SDK_PATH"
    echo "Ready to cross-compile guest/bundle (InfernoMetal.bundle) and guest/driver."
    exit 0
else
    echo "[MISSING] iPhoneOS.sdk (Metal.framework / Foundation.framework / IOKit headers)"
    echo ""
    echo "To build arm64e-apple-ios14.0 guest binaries on Linux or macOS:"
    echo "  1. On macOS with Xcode: xcrun --sdk iphoneos --show-sdk-path"
    echo "  2. On Linux: export IOS_SDK_PATH=/path/to/iPhoneOS14.x.sdk"
    echo "     and compile with: clang++ --target=arm64e-apple-ios14.0 -isysroot \$IOS_SDK_PATH -fuse-ld=lld"
    exit 1
fi
