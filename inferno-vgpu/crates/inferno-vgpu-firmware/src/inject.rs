//! Kernelcache kext symbol verification and prelink injection planner.
//!
//! Validates that every undefined symbol imported by a guest iOS kext is defined
//! by the target iOS kernelcache (like `reims-vgpu/guest/tools/kc-imports.py`)
//! and plans a 16 KiB-aligned segment placement + `__PRELINK_INFO.__info` XML entry.

use crate::macho::{MH_EXECUTE, MH_FILESET, MH_KEXT_BUNDLE, MH_OBJECT, MachoError, ParsedMacho};
use std::fmt;

/// Apple Silicon / ARM64 iOS page granularity (16 KiB).
pub const IOS_PAGE_SIZE: u64 = 0x4000;

/// Error raised during import checking or injection planning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectError {
    /// Mach-O parse failure.
    Macho(MachoError),
    /// Kernelcache is neither `MH_EXECUTE` nor `MH_FILESET`.
    UnsupportedKernelcacheType(u32),
    /// Kext binary is not `arm64`/`arm64e` or wrong Mach-O file type.
    InvalidKextImage(String),
    /// Kext imports symbols that the kernelcache does not define.
    MissingImports(Vec<String>),
    /// Kernelcache has no valid segments.
    EmptyKernelcache,
}

impl fmt::Display for InjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Macho(e) => write!(f, "Mach-O error: {e}"),
            Self::UnsupportedKernelcacheType(ft) => {
                write!(f, "unsupported kernelcache filetype {ft} (expected MH_EXECUTE or MH_FILESET)")
            }
            Self::InvalidKextImage(msg) => write!(f, "invalid kext image: {msg}"),
            Self::MissingImports(missing) => {
                write!(f, "kext imports {} undefined symbol(s): {}", missing.len(), missing.join(", "))
            }
            Self::EmptyKernelcache => write!(f, "kernelcache contains no mapped segments"),
        }
    }
}

impl std::error::Error for InjectError {}

impl From<MachoError> for InjectError {
    fn from(value: MachoError) -> Self {
        Self::Macho(value)
    }
}

/// Result of checking kext undefined imports against a kernelcache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportCheckReport {
    /// Total defined external symbols found in the kernelcache.
    pub kc_defined_count: usize,
    /// Total undefined external symbols imported by the kext.
    pub kext_import_count: usize,
    /// Sorted list of symbols imported by the kext but missing from the kernelcache.
    pub missing_symbols: Vec<String>,
}

/// Planned placement for injecting a kext into an iOS kernelcache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KextInjectionPlan {
    /// Bundle identifier of the kext to inject.
    pub bundle_id: String,
    /// Kernelcache Mach-O file type (`MH_EXECUTE` or `MH_FILESET`).
    pub kc_file_type: u32,
    /// 16 KiB-aligned virtual load address assigned above existing kernel segments.
    pub load_vmaddr: u64,
    /// 16 KiB-aligned total virtual size reserved for the kext image.
    pub mapped_vmsize: u64,
    /// Offset where the kext's `kmod_info` struct will reside (or `0` for `MH_FILESET`).
    pub kmod_info_vmaddr: u64,
    /// Generated XNU `OSUnserializeXML` `<dict>` entry for `__PRELINK_INFO.__info`.
    pub prelink_xml_entry: String,
    /// Import check summary.
    pub imports: ImportCheckReport,
}

/// Checks every undefined external symbol in `kext_bytes` against defined symbols in `kc_bytes`.
pub fn check_kext_imports(
    kc_bytes: &[u8],
    kext_bytes: &[u8],
) -> Result<ImportCheckReport, InjectError> {
    let kc = ParsedMacho::parse(kc_bytes)?;
    if kc.header.file_type != MH_EXECUTE && kc.header.file_type != MH_FILESET {
        return Err(InjectError::UnsupportedKernelcacheType(kc.header.file_type));
    }
    let kext = ParsedMacho::parse(kext_bytes)?;
    if !kext.header.is_arm64() {
        return Err(InjectError::InvalidKextImage(format!(
            "expected CPU_TYPE_ARM64, got 0x{:08x}",
            kext.header.cpu_type
        )));
    }
    if kext.header.file_type != MH_KEXT_BUNDLE && kext.header.file_type != MH_OBJECT {
        return Err(InjectError::InvalidKextImage(format!(
            "expected MH_KEXT_BUNDLE or MH_OBJECT, got {}",
            kext.header.file_type
        )));
    }

    let defined = kc.collect_defined_symbols(kc_bytes)?;
    let imports = kext.collect_undefined_symbols(kext_bytes)?;

    let mut missing: Vec<String> = imports
        .iter()
        .filter(|sym| !defined.contains(*sym))
        .cloned()
        .collect();
    missing.sort();

    Ok(ImportCheckReport {
        kc_defined_count: defined.len(),
        kext_import_count: imports.len(),
        missing_symbols: missing,
    })
}

/// Produces a validated prelink injection plan for a guest iOS kext against `kc_bytes`.
pub fn plan_kext_injection(
    kc_bytes: &[u8],
    kext_bytes: &[u8],
    bundle_id: &str,
    io_class: &str,
    dt_compatible: &str,
) -> Result<KextInjectionPlan, InjectError> {
    let kc = ParsedMacho::parse(kc_bytes)?;
    let imports = check_kext_imports(kc_bytes, kext_bytes)?;
    if !imports.missing_symbols.is_empty() {
        return Err(InjectError::MissingImports(imports.missing_symbols));
    }

    let (_lo, hi) = kc.vm_bounds().ok_or(InjectError::EmptyKernelcache)?;
    let load_vmaddr = align_up(hi, IOS_PAGE_SIZE);
    let mapped_vmsize = align_up(kext_bytes.len() as u64, IOS_PAGE_SIZE).max(IOS_PAGE_SIZE);
    let kmod_info_vmaddr = if kc.header.file_type == MH_EXECUTE {
        load_vmaddr
    } else {
        0
    };

    let prelink_xml_entry = format!(
        "<dict>\
         <key>CFBundleIdentifier</key><string>{bundle_id}</string>\
         <key>CFBundleExecutable</key><string>{io_class}</string>\
         <key>CFBundleVersion</key><string>1.0.0</string>\
         <key>_PrelinkExecutableLoadAddr</key><integer size=\"64\">0x{load_vmaddr:x}</integer>\
         <key>_PrelinkExecutableSize</key><integer size=\"64\">0x{mapped_vmsize:x}</integer>\
         <key>_PrelinkKmodInfo</key><integer size=\"64\">0x{kmod_info_vmaddr:x}</integer>\
         <key>IOKitPersonalities</key><dict>\
           <key>{io_class}</key><dict>\
             <key>CFBundleIdentifier</key><string>{bundle_id}</string>\
             <key>IOClass</key><string>{io_class}</string>\
             <key>IOProviderClass</key><string>AppleARMIODevice</string>\
             <key>IONameMatch</key><string>{dt_compatible}</string>\
           </dict>\
         </dict>\
         </dict>"
    );

    Ok(KextInjectionPlan {
        bundle_id: bundle_id.to_owned(),
        kc_file_type: kc.header.file_type,
        load_vmaddr,
        mapped_vmsize,
        kmod_info_vmaddr,
        prelink_xml_entry,
        imports,
    })
}

const fn align_up(v: u64, align: u64) -> u64 {
    (v + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macho::{
        CPU_SUBTYPE_ARM64E, CPU_TYPE_ARM64, LC_SEGMENT_64, LC_SYMTAB, MH_MAGIC_64,
    };
    use crate::manifest::{
        IVG_FW_ENTRY_BUNDLE, IVG_FW_ENTRY_FLAG_ARM64E, IVG_FW_ENTRY_FLAG_PRELINK,
        IVG_FW_ENTRY_KEXT, IVG_FW_FLAG_ENABLE_METAL_FEATURESET, PackageEntrySpec, build_package,
        verify_package,
    };

    fn make_synthetic_macho(
        file_type: u32,
        seg_vmaddr: u64,
        defined_syms: &[&str],
        undef_syms: &[&str],
        prelink_xml: Option<&str>,
    ) -> Vec<u8> {
        // Header placeholder (32 bytes)
        let mut out = vec![0u8; 32];

        let mut ncmds = 0u32;
        let mut cmds = Vec::new();

        // Segment 1: __TEXT
        ncmds += 1;
        let mut seg = [0u8; 72];
        seg[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        seg[4..8].copy_from_slice(&72u32.to_le_bytes());
        seg[8..14].copy_from_slice(b"__TEXT");
        seg[24..32].copy_from_slice(&seg_vmaddr.to_le_bytes());
        seg[32..40].copy_from_slice(&0x4000u64.to_le_bytes());
        seg[40..48].copy_from_slice(&0u64.to_le_bytes());
        seg[48..56].copy_from_slice(&0x1000u64.to_le_bytes());
        cmds.extend_from_slice(&seg);

        // Optional __PRELINK_INFO.__info
        let xml_offset_placeholder = if prelink_xml.is_some() {
            ncmds += 1;
            let pos = cmds.len();
            let mut pseg = [0u8; 72 + 80];
            pseg[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
            pseg[4..8].copy_from_slice(&(152u32).to_le_bytes());
            pseg[8..22].copy_from_slice(b"__PRELINK_INFO");
            pseg[24..32].copy_from_slice(&(seg_vmaddr + 0x4000).to_le_bytes());
            pseg[32..40].copy_from_slice(&0x4000u64.to_le_bytes());
            pseg[64..68].copy_from_slice(&1u32.to_le_bytes()); // nsects = 1
            pseg[72..78].copy_from_slice(b"__info");
            pseg[88..102].copy_from_slice(b"__PRELINK_INFO");
            pseg[104..112].copy_from_slice(&(seg_vmaddr + 0x4000).to_le_bytes());
            cmds.extend_from_slice(&pseg);
            Some(pos)
        } else {
            None
        };

        // LC_SYMTAB placeholder (24 bytes)
        ncmds += 1;
        let symtab_cmd_pos = cmds.len();
        cmds.resize(cmds.len() + 24, 0);

        let sizeofcmds = cmds.len() as u32;
        out.extend_from_slice(&cmds);

        // Optional XML payload
        if let (Some(xml), Some(pos)) = (prelink_xml, xml_offset_placeholder) {
            let xml_off = out.len() as u32;
            let xml_bytes = xml.as_bytes();
            out.extend_from_slice(xml_bytes);
            let sec_pos = 32 + pos + 72;
            out[sec_pos + 40..sec_pos + 48].copy_from_slice(&(xml_bytes.len() as u64).to_le_bytes());
            out[sec_pos + 48..sec_pos + 52].copy_from_slice(&xml_off.to_le_bytes());
        }

        // Build strtab + nlist_64 table
        let mut strtab = vec![0u8];
        let mut nlists = Vec::new();
        for (i, sym) in defined_syms.iter().enumerate() {
            let strx = strtab.len() as u32;
            strtab.extend_from_slice(sym.as_bytes());
            strtab.push(0);
            let mut nl = [0u8; 16];
            nl[0..4].copy_from_slice(&strx.to_le_bytes());
            nl[4] = 0x0F; // N_SECT | N_EXT
            nl[5] = 1;
            nl[8..16].copy_from_slice(&(seg_vmaddr + 0x100 + (i as u64) * 16).to_le_bytes());
            nlists.extend_from_slice(&nl);
        }
        for sym in undef_syms {
            let strx = strtab.len() as u32;
            strtab.extend_from_slice(sym.as_bytes());
            strtab.push(0);
            let mut nl = [0u8; 16];
            nl[0..4].copy_from_slice(&strx.to_le_bytes());
            nl[4] = 0x01; // N_UNDF | N_EXT
            nlists.extend_from_slice(&nl);
        }

        let symoff = out.len() as u32;
        let nsyms = (defined_syms.len() + undef_syms.len()) as u32;
        out.extend_from_slice(&nlists);
        let stroff = out.len() as u32;
        let strsize = strtab.len() as u32;
        out.extend_from_slice(&strtab);

        // Patch header
        out[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        out[4..8].copy_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
        out[8..12].copy_from_slice(&CPU_SUBTYPE_ARM64E.to_le_bytes());
        out[12..16].copy_from_slice(&file_type.to_le_bytes());
        out[16..20].copy_from_slice(&ncmds.to_le_bytes());
        out[20..24].copy_from_slice(&sizeofcmds.to_le_bytes());

        // Patch LC_SYMTAB
        let sc = 32 + symtab_cmd_pos;
        out[sc..sc + 4].copy_from_slice(&LC_SYMTAB.to_le_bytes());
        out[sc + 4..sc + 8].copy_from_slice(&24u32.to_le_bytes());
        out[sc + 8..sc + 12].copy_from_slice(&symoff.to_le_bytes());
        out[sc + 12..sc + 16].copy_from_slice(&nsyms.to_le_bytes());
        out[sc + 16..sc + 20].copy_from_slice(&stroff.to_le_bytes());
        out[sc + 20..sc + 24].copy_from_slice(&strsize.to_le_bytes());

        out
    }

    #[test]
    fn kext_import_check_and_injection_plan_succeed() {
        let xml = "<dict><key>CFBundleIdentifier</key><string>com.apple.AGXG12P</string></dict>";
        let kc = make_synthetic_macho(
            MH_EXECUTE,
            0xFFFF_FFF0_0700_4000,
            &["_IOLog", "__ZN9IOService5startEPS_", "__ZN12IOUserClient5startEP9IOService"],
            &[],
            Some(xml),
        );
        let kext = make_synthetic_macho(
            MH_KEXT_BUNDLE,
            0,
            &["_InfernoVgpuEntry"],
            &["_IOLog", "__ZN9IOService5startEPS_"],
            None,
        );

        let parsed_kc = ParsedMacho::parse(&kc).unwrap();
        assert_eq!(parsed_kc.prelinked_bundle_ids(&kc), vec!["com.apple.AGXG12P"]);

        let plan = plan_kext_injection(
            &kc,
            &kext,
            "dev.inferno.driver.InfernoVgpuDriver",
            "InfernoVgpuService",
            "apple,inferno-pvgpu",
        )
        .expect("plan succeeds");
        assert_eq!(plan.load_vmaddr, 0xFFFF_FFF0_0700_C000);
        assert_eq!(plan.mapped_vmsize, IOS_PAGE_SIZE);
        assert!(plan.prelink_xml_entry.contains("apple,inferno-pvgpu"));
        assert_eq!(plan.imports.kext_import_count, 2);
        assert!(plan.imports.missing_symbols.is_empty());

        // Missing symbol must be refused loudly.
        let bad_kext = make_synthetic_macho(
            MH_KEXT_BUNDLE,
            0,
            &[],
            &["_IOLog", "_nonexistent_xnu_symbol"],
            None,
        );
        let err = plan_kext_injection(
            &kc,
            &bad_kext,
            "dev.inferno.driver.InfernoVgpuDriver",
            "InfernoVgpuService",
            "apple,inferno-pvgpu",
        )
        .unwrap_err();
        assert_eq!(
            err,
            InjectError::MissingImports(vec!["_nonexistent_xnu_symbol".to_owned()])
        );
    }

    #[test]
    fn ivgf_package_build_verify_and_tamper_detection() {
        let pkg = build_package(
            (14, 0),
            "MTL2,2",
            true,
            &[
                PackageEntrySpec {
                    kind: IVG_FW_ENTRY_KEXT,
                    flags: IVG_FW_ENTRY_FLAG_ARM64E | IVG_FW_ENTRY_FLAG_PRELINK,
                    identifier: "dev.inferno.driver.InfernoVgpuDriver".to_owned(),
                    target_path: String::new(),
                    payload: vec![0xCF, 0xFA, 0xED, 0xFE, 1, 2, 3, 4],
                    xml: b"<dict><key>IONameMatch</key><string>apple,inferno-pvgpu</string></dict>"
                        .to_vec(),
                },
                PackageEntrySpec {
                    kind: IVG_FW_ENTRY_BUNDLE,
                    flags: IVG_FW_ENTRY_FLAG_ARM64E,
                    identifier: "dev.inferno.metal.InfernoMetal".to_owned(),
                    target_path: "/System/Library/Extensions/AGXMetalA13.bundle/AGXMetalA13"
                        .to_owned(),
                    payload: vec![0xCF, 0xFA, 0xED, 0xFE, 9, 8, 7, 6],
                    xml: Vec::new(),
                },
            ],
        )
        .expect("build package");

        let verified = verify_package(&pkg).expect("verify package");
        assert_eq!(verified.header.entry_count, 2);
        assert_ne!(verified.header.flags & IVG_FW_FLAG_ENABLE_METAL_FEATURESET, 0);

        let mut tampered = pkg;
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        assert!(verify_package(&tampered).is_err());
    }
}
