//! `inferno-vgpu-fw`: CLI tool for inspecting iOS kernelcaches, verifying kext
//! symbol imports, and staging/verifying `InfernoVgpu.fw` (`IVGF`) packages.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use inferno_vgpu_firmware::im4p::decode_kernelcache_bytes;
use inferno_vgpu_firmware::inject::{check_kext_imports, plan_kext_injection};
use inferno_vgpu_firmware::macho::{MH_EXECUTE, MH_FILESET, ParsedMacho};
use inferno_vgpu_firmware::manifest::{
    IVG_FW_ENTRY_BUNDLE, IVG_FW_ENTRY_DT_OVERLAY, IVG_FW_ENTRY_FLAG_ARM64E,
    IVG_FW_ENTRY_FLAG_PRELINK, IVG_FW_ENTRY_FLAG_REQUIRED, IVG_FW_ENTRY_KEXT, PackageEntrySpec,
    build_package, cstr_to_string, verify_package,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn print_usage() {
    eprintln!(
        "inferno-vgpu-fw — iOS kernelcache & IVGF firmware package tool\n\
         \n\
         Usage:\n\
         \x20 inferno-vgpu-fw inspect-kc <kernelcache> [--lzfse-dir <dir>]\n\
         \x20 inferno-vgpu-fw check-imports <kernelcache> <kext-macho> [--lzfse-dir <dir>]\n\
         \x20 inferno-vgpu-fw plan-inject <kernelcache> <kext-macho> [--lzfse-dir <dir>]\n\
         \x20 inferno-vgpu-fw stage --out <InfernoVgpu.fw> [--featureset <MTL2,2>] [--enable-metal]\n\
         \x20                       [--kext <id>:<macho>[:<plist>]]\n\
         \x20                       [--bundle <id>:<macho>:<ios-target-path>]\n\
         \x20                       [--dt-overlay <id>:<plist>]\n\
         \x20 inferno-vgpu-fw verify <InfernoVgpu.fw>"
    );
}

fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn load_kc(path: &Path, lzfse_dir: Option<&Path>) -> Result<Vec<u8>, String> {
    let raw = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    decode_kernelcache_bytes(&raw, lzfse_dir)
        .map_err(|e| format!("decode kernelcache {}: {e}", path.display()))
}

fn cmd_inspect_kc(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "inspect-kc requires <kernelcache>".to_owned())?;
    let lzfse_dir = find_flag_value(args, "--lzfse-dir").map(PathBuf::from);
    let kc_bytes = load_kc(Path::new(path), lzfse_dir.as_deref())?;
    let kc = ParsedMacho::parse(&kc_bytes).map_err(|e| format!("parse Mach-O: {e}"))?;

    let ft_name = match kc.header.file_type {
        MH_EXECUTE => "MH_EXECUTE (prelinked iOS 14 style)",
        MH_FILESET => "MH_FILESET (iOS 15+ kernel collection)",
        other => {
            println!("Kernelcache file_type: {other}");
            "OTHER"
        }
    };
    println!("Kernelcache: {path}");
    println!("  Decoded size : {} bytes", kc_bytes.len());
    println!(
        "  Arch         : cpu=0x{:08x} subtype=0x{:08x} (arm64e={})",
        kc.header.cpu_type,
        kc.header.cpu_subtype,
        kc.header.is_arm64e()
    );
    println!("  File type    : {} ({ft_name})", kc.header.file_type);
    if let Some((lo, hi)) = kc.vm_bounds() {
        println!("  VM bounds    : 0x{lo:016x} .. 0x{hi:016x}");
    }
    println!("  Segments     : {}", kc.segments.len());

    let defined = kc
        .collect_defined_symbols(&kc_bytes)
        .map_err(|e| format!("collect symbols: {e}"))?;
    println!("  Defined syms : {}", defined.len());

    let bundles = kc.prelinked_bundle_ids(&kc_bytes);
    println!("  Prelinked    : {} bundle IDs", bundles.len());
    for bid in &bundles {
        let lower = bid.to_ascii_lowercase();
        if lower.contains("agx")
            || lower.contains("accel")
            || lower.contains("surface")
            || lower.contains("mobilegraphics")
            || lower.contains("display")
        {
            println!("    - {bid}");
        }
    }
    Ok(())
}

fn cmd_check_imports(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("check-imports requires <kernelcache> <kext-macho>".to_owned());
    }
    let lzfse_dir = find_flag_value(args, "--lzfse-dir").map(PathBuf::from);
    let kc_bytes = load_kc(Path::new(&args[0]), lzfse_dir.as_deref())?;
    let kext_bytes = fs::read(&args[1]).map_err(|e| format!("read {}: {e}", args[1]))?;
    let report = check_kext_imports(&kc_bytes, &kext_bytes).map_err(|e| e.to_string())?;
    println!(
        "KC defined symbols: {}, kext imports: {}, missing: {}",
        report.kc_defined_count,
        report.kext_import_count,
        report.missing_symbols.len()
    );
    if !report.missing_symbols.is_empty() {
        for sym in &report.missing_symbols {
            eprintln!("  MISSING: {sym}");
        }
        return Err(format!("{} unresolved import(s)", report.missing_symbols.len()));
    }
    Ok(())
}

fn cmd_plan_inject(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("plan-inject requires <kernelcache> <kext-macho>".to_owned());
    }
    let lzfse_dir = find_flag_value(args, "--lzfse-dir").map(PathBuf::from);
    let kc_bytes = load_kc(Path::new(&args[0]), lzfse_dir.as_deref())?;
    let kext_bytes = fs::read(&args[1]).map_err(|e| format!("read {}: {e}", args[1]))?;
    let plan = plan_kext_injection(
        &kc_bytes,
        &kext_bytes,
        "dev.inferno.driver.InfernoVgpuDriver",
        "InfernoVgpuService",
        "apple,inferno-pvgpu",
    )
    .map_err(|e| e.to_string())?;
    println!("Bundle ID   : {}", plan.bundle_id);
    println!("Load VA     : 0x{:016x}", plan.load_vmaddr);
    println!("Mapped size : 0x{:x}", plan.mapped_vmsize);
    println!("Prelink XML : {}", plan.prelink_xml_entry);
    Ok(())
}

fn cmd_stage(args: &[String]) -> Result<(), String> {
    let out_path = find_flag_value(args, "--out")
        .ok_or_else(|| "stage requires --out <InfernoVgpu.fw>".to_owned())?;
    let featureset = find_flag_value(args, "--featureset").unwrap_or_else(|| "GLES2,0".to_owned());
    let enable_metal = args.iter().any(|a| a == "--enable-metal");

    let mut specs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--kext" => {
                let val = args.get(i + 1).ok_or("--kext expects <id>:<macho>[:<plist>]")?;
                let parts: Vec<&str> = val.split(':').collect();
                if parts.len() < 2 || parts.len() > 3 {
                    return Err("--kext format is <id>:<macho>[:<plist>]".to_owned());
                }
                let payload = fs::read(parts[1]).map_err(|e| format!("read {}: {e}", parts[1]))?;
                let xml = if let Some(p) = parts.get(2) {
                    fs::read(p).map_err(|e| format!("read {p}: {e}"))?
                } else {
                    Vec::new()
                };
                specs.push(PackageEntrySpec {
                    kind: IVG_FW_ENTRY_KEXT,
                    flags: IVG_FW_ENTRY_FLAG_ARM64E
                        | IVG_FW_ENTRY_FLAG_PRELINK
                        | IVG_FW_ENTRY_FLAG_REQUIRED,
                    identifier: parts[0].to_owned(),
                    target_path: String::new(),
                    payload,
                    xml,
                });
                i += 2;
            }
            "--bundle" => {
                let val = args
                    .get(i + 1)
                    .ok_or("--bundle expects <id>:<macho>:<ios-target-path>")?;
                let parts: Vec<&str> = val.split(':').collect();
                if parts.len() != 3 {
                    return Err("--bundle format is <id>:<macho>:<ios-target-path>".to_owned());
                }
                let payload = fs::read(parts[1]).map_err(|e| format!("read {}: {e}", parts[1]))?;
                specs.push(PackageEntrySpec {
                    kind: IVG_FW_ENTRY_BUNDLE,
                    flags: IVG_FW_ENTRY_FLAG_ARM64E | IVG_FW_ENTRY_FLAG_REQUIRED,
                    identifier: parts[0].to_owned(),
                    target_path: parts[2].to_owned(),
                    payload,
                    xml: Vec::new(),
                });
                i += 2;
            }
            "--dt-overlay" => {
                let val = args.get(i + 1).ok_or("--dt-overlay expects <id>:<file>")?;
                let parts: Vec<&str> = val.split(':').collect();
                if parts.len() != 2 {
                    return Err("--dt-overlay format is <id>:<file>".to_owned());
                }
                let payload = fs::read(parts[1]).map_err(|e| format!("read {}: {e}", parts[1]))?;
                specs.push(PackageEntrySpec {
                    kind: IVG_FW_ENTRY_DT_OVERLAY,
                    flags: 0,
                    identifier: parts[0].to_owned(),
                    target_path: String::new(),
                    payload,
                    xml: Vec::new(),
                });
                i += 2;
            }
            _ => i += 1,
        }
    }

    let pkg = build_package((14, 0), &featureset, enable_metal, &specs)
        .map_err(|e| format!("build package: {e}"))?;
    if let Some(parent) = Path::new(&out_path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    fs::write(&out_path, &pkg).map_err(|e| format!("write {out_path}: {e}"))?;
    println!(
        "Staged {out_path} ({} bytes, {} entries, featureset={featureset}, enable_metal={enable_metal})",
        pkg.len(),
        specs.len()
    );
    Ok(())
}

fn cmd_verify(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "verify requires <InfernoVgpu.fw>".to_owned())?;
    let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let verified = verify_package(&bytes).map_err(|e| format!("verify {path}: {e}"))?;
    println!(
        "IVGF v{} OK: {} bytes, {} entries, min_ios={}.{}, featureset={}, flags=0x{:x}",
        verified.header.version,
        verified.header.total_size,
        verified.header.entry_count,
        verified.header.min_ios_major,
        verified.header.min_ios_minor,
        cstr_to_string(&verified.header.featureset_class),
        verified.header.flags
    );
    for (idx, e) in verified.entries.iter().enumerate() {
        println!(
            "  [{idx}] kind={} id={} payload={}B xml={}B target={}",
            e.kind,
            cstr_to_string(&e.identifier),
            e.payload_size,
            e.xml_size,
            cstr_to_string(&e.target_path)
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let res = match cmd.as_str() {
        "inspect-kc" => cmd_inspect_kc(rest),
        "check-imports" => cmd_check_imports(rest),
        "plan-inject" => cmd_plan_inject(rest),
        "stage" => cmd_stage(rest),
        "verify" => cmd_verify(rest),
        "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown subcommand: {other}")),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[inferno-vgpu-fw] ERROR: {e}");
            ExitCode::FAILURE
        }
    }
}
