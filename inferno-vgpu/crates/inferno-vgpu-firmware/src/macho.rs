//! 64-bit Mach-O parser for iOS `MH_EXECUTE` (prelinked kernelcache),
//! `MH_FILESET` (iOS 15+ kernel collection), `MH_KEXT_BUNDLE`, and `MH_BUNDLE` / `MH_DYLIB`.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

/// Little-endian 64-bit Mach-O magic (`0xFEEDFACF`).
pub const MH_MAGIC_64: u32 = 0xFEED_FACF;
/// Mach-O CPU type `CPU_TYPE_ARM64`.
pub const CPU_TYPE_ARM64: u32 = 0x0100_000C;
/// Mach-O CPU subtype `CPU_SUBTYPE_ARM64E` mask (low 24 bits == 2).
pub const CPU_SUBTYPE_ARM64E: u32 = 2;

/// Mach-O file types.
pub const MH_OBJECT: u32 = 1;
/// Executable / classic prelinked iOS kernelcache.
pub const MH_EXECUTE: u32 = 2;
/// Dynamic library.
pub const MH_DYLIB: u32 = 6;
/// Loadable bundle (e.g. Metal plugin bundle).
pub const MH_BUNDLE: u32 = 8;
/// Relocatable kernel extension bundle (`MH_KEXT_BUNDLE`).
pub const MH_KEXT_BUNDLE: u32 = 11;
/// iOS 15+ / macOS 11+ kernel collection (`MH_FILESET`).
pub const MH_FILESET: u32 = 12;

/// Load command constants.
pub const LC_SYMTAB: u32 = 0x02;
/// 64-bit segment load command.
pub const LC_SEGMENT_64: u32 = 0x19;
/// Embedded code signature linkedit data (`LC_CODE_SIGNATURE`).
pub const LC_CODE_SIGNATURE: u32 = 0x1D;
/// Build version command (`LC_BUILD_VERSION`).
pub const LC_BUILD_VERSION: u32 = 0x32;
/// Fileset entry command (`LC_FILESET_ENTRY`).
pub const LC_FILESET_ENTRY: u32 = 0x8000_0035;

/// `LC_BUILD_VERSION` platform identifier for iOS.
pub const PLATFORM_IOS: u32 = 2;
/// `LC_BUILD_VERSION` platform identifier for macOS.
pub const PLATFORM_MACOS: u32 = 1;

/// `n_type` bitmasks for `nlist_64`.
const N_STAB: u8 = 0xE0;
const N_TYPE: u8 = 0x0E;
const N_EXT: u8 = 0x01;
const N_UNDF: u8 = 0x00;
const N_SECT: u8 = 0x0E;

/// Error produced while parsing a Mach-O image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachoError {
    /// Input buffer is shorter than required header or load command.
    Truncated(&'static str),
    /// Header magic is not `MH_MAGIC_64`.
    BadMagic(u32),
    /// Load command bounds overflow or have invalid size.
    BadLoadCommand(&'static str),
}

impl fmt::Display for MachoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated(what) => write!(f, "truncated Mach-O ({what})"),
            Self::BadMagic(m) => write!(f, "bad Mach-O magic: 0x{m:08x} (expected 0x{MH_MAGIC_64:08x})"),
            Self::BadLoadCommand(msg) => write!(f, "invalid Mach-O load command: {msg}"),
        }
    }
}

impl std::error::Error for MachoError {}

/// Parsed 64-bit Mach-O header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachoHeader {
    /// CPU type (`CPU_TYPE_ARM64` = `0x0100000C`).
    pub cpu_type: u32,
    /// CPU subtype (low 24 bits: `0` = arm64, `2` = arm64e).
    pub cpu_subtype: u32,
    /// Mach-O file type (`MH_EXECUTE`, `MH_FILESET`, `MH_KEXT_BUNDLE`, ...).
    pub file_type: u32,
    /// Number of load commands.
    pub ncmds: u32,
    /// Total bytes of load commands.
    pub sizeofcmds: u32,
    /// Mach-O flags.
    pub flags: u32,
}

impl MachoHeader {
    /// True if this Mach-O targets 64-bit ARM (`arm64` or `arm64e`).
    pub const fn is_arm64(&self) -> bool {
        self.cpu_type == CPU_TYPE_ARM64
    }

    /// True if this Mach-O targets `arm64e` (PAC-enabled).
    pub const fn is_arm64e(&self) -> bool {
        self.cpu_type == CPU_TYPE_ARM64 && (self.cpu_subtype & 0x00FF_FFFF) == CPU_SUBTYPE_ARM64E
    }
}

/// Parsed 64-bit section descriptor inside an `LC_SEGMENT_64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachoSection {
    /// Section name (e.g. `__info`, `__kmod_info`, `__text`).
    pub sect_name: String,
    /// Owning segment name (e.g. `__PRELINK_INFO`).
    pub seg_name: String,
    /// Virtual address.
    pub addr: u64,
    /// Section size in bytes.
    pub size: u64,
    /// File offset in bytes.
    pub offset: u32,
}

/// Parsed 64-bit segment descriptor (`LC_SEGMENT_64`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachoSegment {
    /// Segment name (e.g. `__TEXT`, `__PRELINK_INFO`).
    pub name: String,
    /// Virtual address.
    pub vmaddr: u64,
    /// Virtual memory size.
    pub vmsize: u64,
    /// File offset.
    pub fileoff: u64,
    /// File size.
    pub filesize: u64,
    /// Maximum VM protection bits.
    pub maxprot: u32,
    /// Initial VM protection bits.
    pub initprot: u32,
    /// Contained sections.
    pub sections: Vec<MachoSection>,
}

/// Entry inside an `MH_FILESET` kernel collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesetEntry {
    /// Bundle identifier (e.g. `com.apple.kernel`, `com.apple.AGXG12P`).
    pub entry_id: String,
    /// Virtual address of entry Mach-O header.
    pub vmaddr: u64,
    /// File offset of entry Mach-O header.
    pub fileoff: u64,
}

/// Parsed `LC_BUILD_VERSION` platform & min OS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildVersion {
    /// Platform ID (`PLATFORM_IOS` = 2, `PLATFORM_MACOS` = 1).
    pub platform: u32,
    /// Encoded min OS (`major << 16 | minor << 8 | patch`).
    pub min_os: u32,
    /// Encoded SDK version.
    pub sdk: u32,
}

/// Parsed summary of a Mach-O image (header, segments, build version, fileset entries, symbol table bounds).
#[derive(Clone, Debug)]
pub struct ParsedMacho {
    /// Mach-O header.
    pub header: MachoHeader,
    /// All `LC_SEGMENT_64` segments.
    pub segments: Vec<MachoSegment>,
    /// Optional `LC_BUILD_VERSION`.
    pub build_version: Option<BuildVersion>,
    /// `LC_FILESET_ENTRY` list (non-empty for `MH_FILESET`).
    pub fileset_entries: Vec<FilesetEntry>,
    /// Symbol table command `(symoff, nsyms, stroff, strsize)` if present.
    pub symtab: Option<(u32, u32, u32, u32)>,
    /// Embedded code signature `(dataoff, datasize)` if present.
    pub code_signature: Option<(u32, u32)>,
}

impl ParsedMacho {
    /// Parses a 64-bit Mach-O starting at byte offset `base` inside `data`.
    pub fn parse_at(data: &[u8], base: usize) -> Result<Self, MachoError> {
        if base.checked_add(32).is_none_or(|e| e > data.len()) {
            return Err(MachoError::Truncated("mach_header_64"));
        }
        let magic = read_u32(data, base)?;
        if magic != MH_MAGIC_64 {
            return Err(MachoError::BadMagic(magic));
        }
        let header = MachoHeader {
            cpu_type: read_u32(data, base + 4)?,
            cpu_subtype: read_u32(data, base + 8)?,
            file_type: read_u32(data, base + 12)?,
            ncmds: read_u32(data, base + 16)?,
            sizeofcmds: read_u32(data, base + 20)?,
            flags: read_u32(data, base + 24)?,
        };

        let cmds_end = base
            .checked_add(32)
            .and_then(|s| s.checked_add(header.sizeofcmds as usize))
            .ok_or(MachoError::BadLoadCommand("sizeofcmds overflow"))?;
        if cmds_end > data.len() {
            return Err(MachoError::Truncated("load commands"));
        }

        let mut segments = Vec::new();
        let mut build_version = None;
        let mut fileset_entries = Vec::new();
        let mut symtab = None;
        let mut code_signature = None;

        let mut cursor = base + 32;
        for _ in 0..header.ncmds {
            if cursor.checked_add(8).is_none_or(|e| e > cmds_end) {
                return Err(MachoError::BadLoadCommand("command header past sizeofcmds"));
            }
            let cmd = read_u32(data, cursor)?;
            let cmdsize = read_u32(data, cursor + 4)? as usize;
            if cmdsize < 8 || cursor.checked_add(cmdsize).is_none_or(|e| e > cmds_end) {
                return Err(MachoError::BadLoadCommand("invalid cmdsize"));
            }

            match cmd {
                LC_SEGMENT_64 => {
                    if cmdsize < 72 {
                        return Err(MachoError::BadLoadCommand("short LC_SEGMENT_64"));
                    }
                    let name = read_fixed_cstr(&data[cursor + 8..cursor + 24]);
                    let vmaddr = read_u64(data, cursor + 24)?;
                    let vmsize = read_u64(data, cursor + 32)?;
                    let fileoff = read_u64(data, cursor + 40)?;
                    let filesize = read_u64(data, cursor + 48)?;
                    let maxprot = read_u32(data, cursor + 56)?;
                    let initprot = read_u32(data, cursor + 60)?;
                    let nsects = read_u32(data, cursor + 64)?;

                    let needed = 72usize
                        .checked_add((nsects as usize).saturating_mul(80))
                        .ok_or(MachoError::BadLoadCommand("section count overflow"))?;
                    if needed > cmdsize {
                        return Err(MachoError::BadLoadCommand("sections exceed LC_SEGMENT_64"));
                    }
                    let mut sections = Vec::with_capacity(nsects as usize);
                    for s_idx in 0..nsects as usize {
                        let s_off = cursor + 72 + s_idx * 80;
                        sections.push(MachoSection {
                            sect_name: read_fixed_cstr(&data[s_off..s_off + 16]),
                            seg_name: read_fixed_cstr(&data[s_off + 16..s_off + 32]),
                            addr: read_u64(data, s_off + 32)?,
                            size: read_u64(data, s_off + 40)?,
                            offset: read_u32(data, s_off + 48)?,
                        });
                    }
                    segments.push(MachoSegment {
                        name,
                        vmaddr,
                        vmsize,
                        fileoff,
                        filesize,
                        maxprot,
                        initprot,
                        sections,
                    });
                }
                LC_SYMTAB => {
                    if cmdsize < 24 {
                        return Err(MachoError::BadLoadCommand("short LC_SYMTAB"));
                    }
                    symtab = Some((
                        read_u32(data, cursor + 8)?,
                        read_u32(data, cursor + 12)?,
                        read_u32(data, cursor + 16)?,
                        read_u32(data, cursor + 20)?,
                    ));
                }
                LC_CODE_SIGNATURE => {
                    if cmdsize >= 16 {
                        code_signature = Some((read_u32(data, cursor + 8)?, read_u32(data, cursor + 12)?));
                    }
                }
                LC_BUILD_VERSION => {
                    if cmdsize >= 24 {
                        build_version = Some(BuildVersion {
                            platform: read_u32(data, cursor + 8)?,
                            min_os: read_u32(data, cursor + 12)?,
                            sdk: read_u32(data, cursor + 16)?,
                        });
                    }
                }
                LC_FILESET_ENTRY => {
                    if cmdsize < 32 {
                        return Err(MachoError::BadLoadCommand("short LC_FILESET_ENTRY"));
                    }
                    let vmaddr = read_u64(data, cursor + 8)?;
                    let fileoff = read_u64(data, cursor + 16)?;
                    let str_off = read_u32(data, cursor + 24)? as usize;
                    if str_off >= cmdsize {
                        return Err(MachoError::BadLoadCommand("fileset entry_id out of bounds"));
                    }
                    let entry_id = read_fixed_cstr(&data[cursor + str_off..cursor + cmdsize]);
                    fileset_entries.push(FilesetEntry {
                        entry_id,
                        vmaddr,
                        fileoff,
                    });
                }
                _ => {}
            }

            cursor += cmdsize;
        }

        Ok(Self {
            header,
            segments,
            build_version,
            fileset_entries,
            symtab,
            code_signature,
        })
    }

    /// Parses a Mach-O starting at offset `0`.
    pub fn parse(data: &[u8]) -> Result<Self, MachoError> {
        Self::parse_at(data, 0)
    }

    /// Finds a section by `(segment_name, section_name)`.
    pub fn find_section(&self, seg: &str, sect: &str) -> Option<&MachoSection> {
        self.segments
            .iter()
            .find(|s| s.name == seg)
            .and_then(|s| s.sections.iter().find(|sec| sec.sect_name == sect))
    }

    /// Returns the `__TEXT` base virtual address (or first non-zero fileoff-0 segment).
    pub fn text_base(&self) -> Option<u64> {
        self.segments
            .iter()
            .find(|s| s.name == "__TEXT" && s.vmsize > 0)
            .or_else(|| {
                self.segments
                    .iter()
                    .find(|s| s.fileoff == 0 && s.filesize > 0 && s.name != "__PAGEZERO")
            })
            .map(|s| s.vmaddr)
    }

    /// Maps a virtual address `va` inside mapped file bytes to a file offset.
    pub fn va_to_file_offset(&self, va: u64) -> Option<usize> {
        for seg in &self.segments {
            if seg.filesize == 0 {
                continue;
            }
            if let Some(end) = seg.vmaddr.checked_add(seg.filesize)
                && va >= seg.vmaddr
                && va < end
            {
                let delta = va - seg.vmaddr;
                return usize::try_from(seg.fileoff.checked_add(delta)?).ok();
            }
        }
        None
    }

    /// Collects `(symbol_name -> virtual_address)` for defined non-STAB external symbols.
    pub fn collect_defined_symbol_map(&self, data: &[u8]) -> Result<BTreeMap<String, u64>, MachoError> {
        let mut out = BTreeMap::new();
        self.collect_sym_map_into(data, false, &mut out)?;
        if self.header.file_type == MH_FILESET {
            for entry in &self.fileset_entries {
                let off = usize::try_from(entry.fileoff)
                    .map_err(|_| MachoError::BadLoadCommand("fileset fileoff overflow"))?;
                if let Ok(sub) = Self::parse_at(data, off) {
                    sub.collect_sym_map_into(data, false, &mut out)?;
                }
            }
        }
        Ok(out)
    }

    /// Collects `(symbol_name -> virtual_address)` including internal (`!N_EXT`) section symbols.
    pub fn collect_all_defined_symbol_map(
        &self,
        data: &[u8],
    ) -> Result<BTreeMap<String, u64>, MachoError> {
        let mut out = BTreeMap::new();
        self.collect_sym_map_into(data, true, &mut out)?;
        Ok(out)
    }

    fn collect_sym_map_into(
        &self,
        data: &[u8],
        include_internal: bool,
        out: &mut BTreeMap<String, u64>,
    ) -> Result<(), MachoError> {
        let Some((symoff, nsyms, stroff, strsize)) = self.symtab else {
            return Ok(());
        };
        let sym_base = symoff as usize;
        let str_base = stroff as usize;
        let str_end = str_base
            .checked_add(strsize as usize)
            .ok_or(MachoError::Truncated("strtab overflow"))?;
        if str_end > data.len() {
            return Err(MachoError::Truncated("strtab"));
        }
        let strtab = &data[str_base..str_end];
        let sym_bytes = (nsyms as usize)
            .checked_mul(16)
            .ok_or(MachoError::Truncated("symtab overflow"))?;
        if sym_base.checked_add(sym_bytes).is_none_or(|e| e > data.len()) {
            return Err(MachoError::Truncated("symtab"));
        }
        for i in 0..nsyms as usize {
            let off = sym_base + i * 16;
            let n_strx = read_u32(data, off)? as usize;
            let n_type = data[off + 4];
            let n_value = read_u64(data, off + 8)?;
            if (n_type & N_STAB) != 0 || (!include_internal && (n_type & N_EXT) == 0) {
                continue;
            }
            if (n_type & N_TYPE) != N_SECT || n_value == 0 || n_strx >= strtab.len() {
                continue;
            }
            let name = read_fixed_cstr(&strtab[n_strx..]);
            if !name.is_empty() {
                out.entry(name).or_insert(n_value);
            }
        }
        Ok(())
    }

    /// Returns `(lowest_vmaddr, highest_vmaddr_end)` across all non-empty segments.
    pub fn vm_bounds(&self) -> Option<(u64, u64)> {
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        for seg in &self.segments {
            if seg.vmsize == 0 {
                continue;
            }
            lo = lo.min(seg.vmaddr);
            hi = hi.max(seg.vmaddr.saturating_add(seg.vmsize));
        }
        (lo != u64::MAX).then_some((lo, hi))
    }

    /// Collects all defined external symbol names from this image (and all `MH_FILESET` children).
    pub fn collect_defined_symbols(&self, data: &[u8]) -> Result<HashSet<String>, MachoError> {
        let mut out = HashSet::new();
        self.collect_syms_into(data, true, &mut out)?;
        if self.header.file_type == MH_FILESET {
            for entry in &self.fileset_entries {
                let off = usize::try_from(entry.fileoff)
                    .map_err(|_| MachoError::BadLoadCommand("fileset fileoff overflow"))?;
                if let Ok(sub) = Self::parse_at(data, off) {
                    sub.collect_syms_into(data, true, &mut out)?;
                }
            }
        }
        Ok(out)
    }

    /// Collects all external undefined symbol names (imports) referenced by this image.
    pub fn collect_undefined_symbols(&self, data: &[u8]) -> Result<HashSet<String>, MachoError> {
        let mut out = HashSet::new();
        self.collect_syms_into(data, false, &mut out)?;
        Ok(out)
    }

    fn collect_syms_into(
        &self,
        data: &[u8],
        want_defined: bool,
        out: &mut HashSet<String>,
    ) -> Result<(), MachoError> {
        let Some((symoff, nsyms, stroff, strsize)) = self.symtab else {
            return Ok(());
        };
        let sym_base = symoff as usize;
        let str_base = stroff as usize;
        let str_end = str_base
            .checked_add(strsize as usize)
            .ok_or(MachoError::Truncated("strtab overflow"))?;
        if str_end > data.len() {
            return Err(MachoError::Truncated("strtab"));
        }
        let strtab = &data[str_base..str_end];

        let sym_bytes = (nsyms as usize)
            .checked_mul(16)
            .ok_or(MachoError::Truncated("symtab overflow"))?;
        if sym_base.checked_add(sym_bytes).is_none_or(|e| e > data.len()) {
            return Err(MachoError::Truncated("symtab"));
        }

        for i in 0..nsyms as usize {
            let off = sym_base + i * 16;
            let n_strx = read_u32(data, off)? as usize;
            let n_type = data[off + 4];
            let n_value = read_u64(data, off + 8)?;

            if (n_type & N_STAB) != 0 || (n_type & N_EXT) == 0 {
                continue;
            }
            let kind = n_type & N_TYPE;
            let matches = if want_defined {
                kind == N_SECT && n_value != 0
            } else {
                kind == N_UNDF && n_value == 0
            };
            if !matches || n_strx >= strtab.len() {
                continue;
            }
            let name = read_fixed_cstr(&strtab[n_strx..]);
            if !name.is_empty() {
                out.insert(name);
            }
        }
        Ok(())
    }

    /// Extracts prelinked `CFBundleIdentifier` entries from `__PRELINK_INFO.__info` (iOS 14 `MH_EXECUTE`)
    /// or `LC_FILESET_ENTRY` (iOS 15+ `MH_FILESET`).
    pub fn prelinked_bundle_ids(&self, data: &[u8]) -> Vec<String> {
        if self.header.file_type == MH_FILESET && !self.fileset_entries.is_empty() {
            return self
                .fileset_entries
                .iter()
                .map(|e| e.entry_id.clone())
                .collect();
        }
        let Some(sec) = self.find_section("__PRELINK_INFO", "__info") else {
            return Vec::new();
        };
        let start = sec.offset as usize;
        let end = start.saturating_add(sec.size as usize).min(data.len());
        if start >= end {
            return Vec::new();
        }
        let xml = String::from_utf8_lossy(&data[start..end]);
        extract_cf_bundle_identifiers(&xml)
    }
}

/// Extracts `<key>CFBundleIdentifier</key><string>...</string>` values from XNU `OSUnserializeXML`.
pub fn extract_cf_bundle_identifiers(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = "<key>CFBundleIdentifier</key>";
    let mut rest = xml;
    while let Some(idx) = rest.find(needle) {
        rest = &rest[idx + needle.len()..];
        let Some(s_start) = rest.find("<string") else {
            break;
        };
        rest = &rest[s_start..];
        let Some(gt) = rest.find('>') else {
            break;
        };
        rest = &rest[gt + 1..];
        let Some(s_end) = rest.find("</string>") else {
            break;
        };
        let id = rest[..s_end].trim();
        if !id.is_empty() && !out.iter().any(|existing| existing == id) {
            out.push(id.to_owned());
        }
        rest = &rest[s_end + "</string>".len()..];
    }
    out
}

fn read_u32(data: &[u8], off: usize) -> Result<u32, MachoError> {
    let slice = data
        .get(off..off + 4)
        .ok_or(MachoError::Truncated("u32"))?;
    let mut arr = [0u8; 4];
    arr.copy_from_slice(slice);
    Ok(u32::from_le_bytes(arr))
}

fn read_u64(data: &[u8], off: usize) -> Result<u64, MachoError> {
    let slice = data
        .get(off..off + 8)
        .ok_or(MachoError::Truncated("u64"))?;
    let mut arr = [0u8; 8];
    arr.copy_from_slice(slice);
    Ok(u64::from_le_bytes(arr))
}

fn read_fixed_cstr(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}
