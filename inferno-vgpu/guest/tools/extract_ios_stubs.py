#!/usr/bin/env python3
"""Generate iOS 14.0 arm64/arm64e .tbd v4 linker stubs for Linux ld64.lld cross-builds."""

from pathlib import Path
import sys


def write_tbd(base: Path, rel: str, install_name: str, symbols=None, objc_classes=None, reexports=None):
    p = base / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "--- !tapi-tbd",
        "tbd-version:     4",
        "targets:         [ arm64-ios, arm64e-ios ]",
        f"install-name:    '{install_name}'",
        "exports:",
        "  - targets:         [ arm64-ios, arm64e-ios ]",
    ]
    if reexports:
        lines.append("    re-exports:      [ " + ", ".join(f"'{r}'" for r in reexports) + " ]")
    if symbols:
        lines.append("    symbols:         [ " + ", ".join(f"'{s}'" for s in symbols) + " ]")
    if objc_classes:
        lines.append("    objc-classes:    [ " + ", ".join(objc_classes) + " ]")
    lines.append("...")
    p.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "build" / "sdk-stubs"

    write_tbd(
        out_dir,
        "usr/lib/libSystem.B.tbd",
        "/usr/lib/libSystem.B.dylib",
        symbols=[
            "dyld_stub_binder", "___stack_chk_fail", "___stack_chk_guard", "__NSConcreteGlobalBlock", "__NSConcreteStackBlock",
            "_abort", "_bzero", "_calloc", "_close", "_exit", "_free", "_getenv", "_getpid",
            "_mach_task_self_", "_malloc", "_memcmp", "_memcpy", "_memmove", "_memset",
            "_pthread_mutex_destroy", "_pthread_mutex_init", "_pthread_mutex_lock", "_pthread_mutex_unlock",
            "_realloc", "_sched_yield", "_snprintf", "_strcmp", "_strlen", "_strncpy",
            "_usleep", "__ZdlPv", "__Znwm", "__ZdaPv", "__Znam", "__ZSt9terminatev",
            "___gxx_personality_v0", "___cxa_pure_virtual", "___cxa_guard_acquire", "___cxa_guard_release",
            "_dispatch_once",
        ],
    )
    write_tbd(
        out_dir,
        "usr/lib/libc++.1.tbd",
        "/usr/lib/libc++.1.dylib",
        symbols=[
            "__ZdlPv", "__Znwm", "__ZdaPv", "__Znam",
            "__ZNSt3__112__next_primeEm", "__ZNKSt3__121__basic_string_commonILb1EE20__throw_length_errorEv",
            "__ZNSt3__16thread6detachEv", "__ZNSt3__111this_thread9sleep_forERKNS_6chrono8durationIxNS_5ratioILl1ELl1000000000EEEEE",
            "__ZNSt3__15mutex4lockEv", "__ZNSt3__15mutex6unlockEv", "__ZNSt3__15mutexD1Ev",
        ],
    )
    write_tbd(
        out_dir,
        "usr/lib/libobjc.A.tbd",
        "/usr/lib/libobjc.A.dylib",
        symbols=[
            "_objc_alloc", "_objc_alloc_init", "_objc_autorelease", "_objc_autoreleasePoolPop",
            "_objc_autoreleasePoolPush", "_objc_autoreleaseReturnValue", "_objc_getClass",
            "_objc_msgSend", "_objc_msgSendSuper2", "_objc_release", "_objc_retain",
            "_objc_retainAutoreleasedReturnValue", "_objc_storeStrong", "_objc_storeWeak",
            "_objc_loadWeakRetained", "_objc_destroyWeak", "_sel_registerName",
        ],
        objc_classes=["NSObject"],
    )
    write_tbd(
        out_dir,
        "System/Library/Frameworks/IOKit.framework/IOKit.tbd",
        "/System/Library/Frameworks/IOKit.framework/Versions/A/IOKit",
        symbols=[
            "_kIOMasterPortDefault", "_IOServiceMatching", "_IOServiceGetMatchingService",
            "_IOServiceOpen", "_IOServiceClose", "_IOObjectRelease", "_IOObjectRetain",
            "_IOConnectMapMemory", "_IOConnectUnmapMemory", "_IOConnectCallScalarMethod",
            "_IOConnectCallStructMethod", "_IOConnectCallMethod",
            "_IORegistryEntryCreateCFProperty", "_IORegistryEntrySetCFProperties",
        ],
    )
    write_tbd(
        out_dir,
        "System/Library/Frameworks/IOSurface.framework/IOSurface.tbd",
        "/System/Library/Frameworks/IOSurface.framework/IOSurface",
        symbols=[
            "_IOSurfaceGetID", "_IOSurfaceLookup", "_IOSurfaceCreate",
            "_IOSurfaceGetWidth", "_IOSurfaceGetHeight", "_IOSurfaceGetBytesPerRow",
            "_IOSurfaceGetAllocSize", "_IOSurfaceGetPixelFormat", "_IOSurfaceGetBaseAddress",
            "_IOSurfaceLock", "_IOSurfaceUnlock",
        ],
        objc_classes=["IOSurface"],
    )
    write_tbd(
        out_dir,
        "System/Library/Frameworks/Foundation.framework/Foundation.tbd",
        "/System/Library/Frameworks/Foundation.framework/Foundation",
        symbols=["_NSLog"],
        objc_classes=["NSString", "NSData", "NSDictionary", "NSArray", "NSNumber", "NSError"],
    )
    write_tbd(
        out_dir,
        "System/Library/Frameworks/Metal.framework/Metal.tbd",
        "/System/Library/Frameworks/Metal.framework/Metal",
        symbols=["_MTLCreateSystemDefaultDevice", "_MTLCopyAllDevices"],
        objc_classes=["MTLCompileOptions", "MTLTextureDescriptor", "MTLRenderPassDescriptor", "MTLRenderPipelineDescriptor"],
    )
    write_tbd(
        out_dir,
        "System/Library/PrivateFrameworks/IOGPU.framework/IOGPU.tbd",
        "/System/Library/PrivateFrameworks/IOGPU.framework/IOGPU",
        symbols=["_IOGPUResourceCreate", "_IOGPUCommandQueueCreate"],
        objc_classes=["IOGPUMetalDevice", "IOGPUMetalCommandQueue", "IOGPUMetalCommandBuffer", "IOGPUMetalResource"],
    )
    write_tbd(
        out_dir,
        "System/Library/Frameworks/QuartzCore.framework/QuartzCore.tbd",
        "/System/Library/Frameworks/QuartzCore.framework/QuartzCore",
        symbols=["_CACurrentMediaTime"],
        objc_classes=["CAMetalLayer", "CALayer"],
    )
    print(f"[extract_ios_stubs] generated iOS 14.0 arm64/arm64e .tbd stubs -> {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
