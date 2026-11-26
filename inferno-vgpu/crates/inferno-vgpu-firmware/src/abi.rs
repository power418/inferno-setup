//! iOS kernelcache ABI extractor (`kernelcache.research` -> `iogpu_vtables.json`,
//! `kpi_symbols.toml`, `iogpu_abi.h`) and stripped release symbol matcher (`reloc-map.json`/`bin`).

use crate::macho::ParsedMacho;
use crate::manifest::{IVG_FW_RELOC_ENTRY_SIZE, IvgFwRelocEntry};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use zerocopy::IntoBytes;

/// Core XNU / IOKit / IOGPU symbols required by `InfernoVgpuDriver.kext`.
pub const CORE_KPI_SYMBOLS: &[&str] = &[
    "_IOLog",
    "__ZN9IOService5startEPS_",
    "__ZN9IOService4stopEPS_",
    "__ZN9IOService15registerServiceEj",
    "__ZN9IOService24getDeviceMemoryWithIndexEj",
    "__ZN9IOServiceC2EPK11OSMetaClass",
    "__ZN9IOServiceD2Ev",
    "__ZN12IOUserClient12initWithTaskEP4taskPvjP12OSDictionary",
    "__ZN12IOUserClient5startEP9IOService",
    "__ZN12IOUserClient14externalMethodEjP25IOExternalMethodArgumentsP24IOExternalMethodDispatchP8OSObjectPv",
    "__ZN12IOUserClient19clientMemoryForTypeEjPjPP18IOMemoryDescriptor",
    "__ZN12IOUserClientC2EPK11OSMetaClass",
    "__ZN12IOUserClientD2Ev",
    "__ZN11OSMetaClassC2EPKcPKS_j",
    "__ZN11OSMetaClassD2Ev",
    "__ZNK8OSObject7releaseEv",
    "__ZNK8OSObject6retainEv",
    "__ZN18IOMemoryDescriptor3mapEj",
    "__ZN22IOInterruptEventSource20interruptEventSourceEP8OSObjectPFvS1_PS_iEP9IOServicei",
    "__ZN5IOGPU5startEP9IOService",
    "__ZN5IOGPU4stopEP9IOService",
];

/// Classes whose vtables are decoded from `kernelcache.research` `__DATA_CONST`.
pub const VTABLE_CLASSES: &[(&str, &str)] = &[
    ("IOGPU", "__ZTV5IOGPU"),
    ("IOGPUDevice", "__ZTV11IOGPUDevice"),
    ("IOGPUDeviceUserClient", "__ZTV21IOGPUDeviceUserClient"),
    ("AGXAccelerator", "__ZTV14AGXAccelerator"),
    ("AGXAcceleratorG12P_B0", "__ZTV21AGXAcceleratorG12P_B0"),
    ("IOService", "__ZTV9IOService"),
    ("IOUserClient", "__ZTV12IOUserClient"),
    ("IOSurface", "__ZTV9IOSurface"),
];

/// One decoded slot in an XNU/IOGPU C++ vtable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtableSlot {
    /// Zero-based qword slot index from `__ZTV<Class>` base.
    pub index: usize,
    /// Decoded target virtual address (`text_base + (raw & 0xFFFFFFFF)`).
    pub va: u64,
    /// Resolved symbol name if present in `LC_SYMTAB`.
    pub symbol: Option<String>,
}

/// Extracted ABI bundle from an unstripped `kernelcache.research` image.
#[derive(Clone, Debug)]
pub struct ExtractedAbi {
    /// Text base virtual address.
    pub text_base: u64,
    /// Total defined external/internal symbols found in `LC_SYMTAB`.
    pub total_symbols: usize,
    /// Resolved addresses for `CORE_KPI_SYMBOLS`.
    pub kpi_symbols: BTreeMap<String, u64>,
    /// Decoded class vtables keyed by class name.
    pub vtables: BTreeMap<String, Vec<VtableSlot>>,
}

/// One matched relocation target between `research` and `release` kernelcaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseRelocMatch {
    /// Mach-O symbol name.
    pub symbol: String,
    /// Virtual address in `research` kernelcache.
    pub research_va: u64,
    /// Unique virtual address in stripped `release` kernelcache (`0` if unresolved/ambiguous).
    pub release_va: u64,
    /// Match status (`"unique"`, `"ambiguous"`, or `"missing"`).
    pub status: String,
    /// Candidate hit count during masked search.
    pub hit_count: usize,
    /// Number of ARM64 instructions matched.
    pub window_words: usize,
}

/// Complete release symbol match result ready for JSON + binary `IvgFwRelocEntry` export.
#[derive(Clone, Debug)]
pub struct ReleaseRelocMap {
    /// Per-symbol match entries.
    pub entries: Vec<ReleaseRelocMatch>,
    /// Number of uniquely resolved entries.
    pub unique_count: usize,
}

/// Masks position-dependent immediates in an AArch64 instruction word for cross-build matching.
pub const fn mask_arm64_insn(w: u32) -> u32 {
    let op = w & 0xFC00_0000;
    if op == 0x1400_0000 || op == 0x9400_0000 {
        return w & 0xFC00_0000; /* B / BL imm26 */
    }
    if (w & 0x9F00_0000) == 0x9000_0000 || (w & 0x9F00_0000) == 0x1000_0000 {
        return w & 0x9F00_001F; /* ADRP / ADR imm */
    }
    if (w & 0x7F80_0000) == 0x1100_0000 {
        return w & 0xFFC0_03FF; /* ADD/SUB immediate imm12 (often paired with ADRP) */
    }
    if (w & 0x3B00_0000) == 0x3900_0000 {
        return w & 0xFFC0_03FF; /* LDR/STR unsigned imm12 (often paired with ADRP) */
    }
    if (w & 0xFF00_0010) == 0x5400_0000 || (w & 0x7E00_0000) == 0x3400_0000 {
        return w & 0xFF00_001F; /* B.cond / CBZ / CBNZ */
    }
    if (w & 0x7E00_0000) == 0x3600_0000 {
        return w & 0xFFF8_001F; /* TBZ / TBNZ */
    }
    w
}

fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64_le(data: &[u8], off: usize) -> Option<u64> {
    let b = data.get(off..off + 8)?;
    Some(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

/// Extracts KPI symbols and C++ vtable layouts from an unstripped `kernelcache.research` image.
pub fn extract_research_abi(kc_bytes: &[u8]) -> Result<ExtractedAbi, String> {
    let parsed = ParsedMacho::parse(kc_bytes).map_err(|e| format!("parse research KC: {e}"))?;
    let text_base = parsed
        .text_base()
        .ok_or_else(|| "research KC missing __TEXT segment".to_owned())?;
    let sym_map = parsed
        .collect_all_defined_symbol_map(kc_bytes)
        .map_err(|e| format!("collect symbols: {e}"))?;
    if sym_map.is_empty() {
        return Err("research kernelcache has 0 LC_SYMTAB symbols".to_owned());
    }

    let mut va_to_sym: HashMap<u64, String> = HashMap::with_capacity(sym_map.len());
    for (name, &va) in &sym_map {
        va_to_sym.entry(va).or_insert_with(|| name.clone());
    }

    let mut kpi_symbols = BTreeMap::new();
    for &sym in CORE_KPI_SYMBOLS {
        if let Some(&va) = sym_map.get(sym) {
            kpi_symbols.insert(sym.to_owned(), va);
        }
    }

    let mut vtables = BTreeMap::new();
    for &(cls, ztv) in VTABLE_CLASSES {
        let Some(&ztv_va) = sym_map.get(ztv) else {
            continue;
        };
        let Some(start_off) = parsed.va_to_file_offset(ztv_va) else {
            continue;
        };
        let mut slots = Vec::new();
        let mut invalid_streak = 0usize;
        for idx in 0..512usize {
            let slot_va = ztv_va + (idx as u64) * 8;
            if idx >= 2
                && let Some(other) = va_to_sym.get(&slot_va)
                && other.starts_with("__ZTV")
            {
                break;
            }
            let Some(raw) = read_u64_le(kc_bytes, start_off + idx * 8) else {
                break;
            };
            if raw == 0 {
                slots.push(VtableSlot {
                    index: idx,
                    va: 0,
                    symbol: None,
                });
                if idx >= 2 {
                    invalid_streak += 1;
                    if invalid_streak >= 2 {
                        break;
                    }
                }
                continue;
            }
            let cand = text_base.saturating_add(raw & 0xFFFF_FFFF);
            let sym_opt = va_to_sym
                .get(&cand)
                .cloned()
                .or_else(|| va_to_sym.get(&raw).cloned());
            if idx >= 2 && sym_opt.is_none() {
                invalid_streak += 1;
                if invalid_streak >= 2 {
                    break;
                }
            } else {
                invalid_streak = 0;
            }
            let resolved_va = if va_to_sym.contains_key(&raw) { raw } else { cand };
            slots.push(VtableSlot {
                index: idx,
                va: resolved_va,
                symbol: sym_opt,
            });
        }
        vtables.insert(cls.to_owned(), slots);
    }

    Ok(ExtractedAbi {
        text_base,
        total_symbols: sym_map.len(),
        kpi_symbols,
        vtables,
    })
}

impl ExtractedAbi {
    /// Formats `iogpu_vtables.json`.
    pub fn to_vtables_json(&self) -> String {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"text_base\": \"0x{:016x}\",", self.text_base);
        let _ = writeln!(out, "  \"total_symbols\": {},", self.total_symbols);
        out.push_str("  \"vtables\": {\n");
        for (c_idx, (cls, slots)) in self.vtables.iter().enumerate() {
            let _ = writeln!(out, "    \"{}\": [", json_escape(cls));
            for (s_idx, s) in slots.iter().enumerate() {
                let sym = s
                    .symbol
                    .as_deref()
                    .map(|v| format!("\"{}\"", json_escape(v)))
                    .unwrap_or_else(|| "null".to_owned());
                let comma = if s_idx + 1 < slots.len() { "," } else { "" };
                let _ = writeln!(
                    out,
                    "      {{\"slot\": {}, \"va\": \"0x{:016x}\", \"symbol\": {sym}}}{comma}",
                    s.index, s.va
                );
            }
            let comma = if c_idx + 1 < self.vtables.len() { "," } else { "" };
            let _ = writeln!(out, "    ]{comma}");
        }
        out.push_str("  }\n}\n");
        out
    }

    /// Formats `kpi_symbols.toml`.
    pub fn to_kpi_toml(&self) -> String {
        let mut out = String::from("# Auto-generated by inferno-vgpu-fw extract-abi\n");
        let _ = writeln!(out, "text_base = \"0x{:016x}\"", self.text_base);
        let _ = writeln!(out, "total_symbols = {}", self.total_symbols);
        out.push_str("\n[kpi_symbols]\n");
        for (sym, va) in &self.kpi_symbols {
            let _ = writeln!(out, "\"{}\" = \"0x{:016x}\"", sym, va);
        }
        out
    }

    /// Formats `iogpu_abi.h` with verified slot counts and key vtable slot constants.
    pub fn to_c_header(&self) -> String {
        let mut out = String::from(
            "/* Auto-generated by inferno-vgpu-fw extract-abi (iOS 14.0 18A5351d). */\n\
             #ifndef INFERNO_IOS14_IOGPU_ABI_H\n\
             #define INFERNO_IOS14_IOGPU_ABI_H\n\n",
        );
        for (cls, slots) in &self.vtables {
            let upper = cls.to_ascii_uppercase();
            let _ = writeln!(out, "#define IOS14_{upper}_VTABLE_SLOTS {}u", slots.len());
            for s in slots {
                if let Some(sym) = &s.symbol {
                    if sym.ends_with("5startEP9IOService") {
                        let _ = writeln!(out, "#define IOS14_{upper}_SLOT_START {}u", s.index);
                    } else if sym.ends_with("4stopEP9IOService") {
                        let _ = writeln!(out, "#define IOS14_{upper}_SLOT_STOP {}u", s.index);
                    } else if sym.ends_with("externalMethodEjP25IOExternalMethodArgumentsP24IOExternalMethodDispatchP8OSObjectPv") {
                        let _ = writeln!(out, "#define IOS14_{upper}_SLOT_EXTERNAL_METHOD {}u", s.index);
                    } else if sym.ends_with("clientMemoryForTypeEjPjPP18IOMemoryDescriptor") {
                        let _ = writeln!(out, "#define IOS14_{upper}_SLOT_CLIENT_MEMORY {}u", s.index);
                    }
                }
            }
        }
        out.push_str("\n#endif /* INFERNO_IOS14_IOGPU_ABI_H */\n");
        out
    }
}

/// Matches `CORE_KPI_SYMBOLS` from `research_kc` into stripped `release_kc` using masked instructions.
pub fn match_release_symbols(
    research_kc: &[u8],
    release_kc: &[u8],
) -> Result<ReleaseRelocMap, String> {
    let res = ParsedMacho::parse(research_kc).map_err(|e| format!("parse research KC: {e}"))?;
    let rel = ParsedMacho::parse(release_kc).map_err(|e| format!("parse release KC: {e}"))?;

    let sym_map = res
        .collect_all_defined_symbol_map(research_kc)
        .map_err(|e| format!("collect research symbols: {e}"))?;
    let rel_exec = rel
        .segments
        .iter()
        .find(|s| s.name == "__TEXT_EXEC" && s.filesize > 0)
        .ok_or_else(|| "release KC missing __TEXT_EXEC".to_owned())?;
    let rel_base = rel_exec.fileoff as usize;
    let rel_len = (rel_exec.filesize as usize) & !3;
    let rel_bytes = release_kc
        .get(rel_base..rel_base + rel_len)
        .ok_or_else(|| "release KC __TEXT_EXEC truncated".to_owned())?;

    let mut rel_masked = Vec::with_capacity(rel_len / 4);
    for &chunk in rel_bytes.as_chunks::<4>().0 {
        let w = u32::from_le_bytes(chunk);
        rel_masked.push(mask_arm64_insn(w));
    }

    let mut idx4: HashMap<[u32; 4], Vec<usize>> = HashMap::with_capacity(rel_masked.len() / 2);
    for (i, win) in rel_masked.windows(4).enumerate() {
        idx4.entry([win[0], win[1], win[2], win[3]])
            .or_default()
            .push(i);
    }

    let mut entries = Vec::new();
    let mut unique_count = 0usize;
    for &sym in CORE_KPI_SYMBOLS {
        let Some(&res_va) = sym_map.get(sym) else {
            entries.push(ReleaseRelocMatch {
                symbol: sym.to_owned(),
                research_va: 0,
                release_va: 0,
                status: "missing".to_owned(),
                hit_count: 0,
                window_words: 0,
            });
            continue;
        };
        let Some(res_off) = res.va_to_file_offset(res_va) else {
            continue;
        };
        let mut pat = Vec::with_capacity(24);
        for k in 0..24usize {
            if let Some(w) = read_u32_le(research_kc, res_off + k * 4) {
                pat.push(mask_arm64_insn(w));
            }
        }
        if pat.len() < 8 {
            continue;
        }
        let key = [pat[0], pat[1], pat[2], pat[3]];
        let cands = idx4.get(&key).map(Vec::as_slice).unwrap_or(&[]);

        let mut matched_va = 0u64;
        let mut status = "missing";
        let mut best_hits = 0usize;
        let mut best_win = 0usize;

        for &win in &[12usize, 16usize, 24usize] {
            if pat.len() < win {
                continue;
            }
            let hits: Vec<usize> = cands
                .iter()
                .copied()
                .filter(|&pos| {
                    pos + win <= rel_masked.len() && rel_masked[pos..pos + win] == pat[..win]
                })
                .collect();
            best_hits = hits.len();
            best_win = win;
            if hits.len() == 1 {
                matched_va = rel_exec.vmaddr + (hits[0] as u64) * 4;
                status = "unique";
                unique_count += 1;
                break;
            } else if !hits.is_empty() {
                status = "ambiguous";
            }
        }

        entries.push(ReleaseRelocMatch {
            symbol: sym.to_owned(),
            research_va: res_va,
            release_va: matched_va,
            status: status.to_owned(),
            hit_count: best_hits,
            window_words: best_win,
        });
    }

    Ok(ReleaseRelocMap {
        entries,
        unique_count,
    })
}

impl ReleaseRelocMap {
    /// Formats `reloc-map.json`.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"unique_count\": {},", self.unique_count);
        let _ = writeln!(out, "  \"total_count\": {},", self.entries.len());
        out.push_str("  \"entries\": [\n");
        for (i, e) in self.entries.iter().enumerate() {
            let comma = if i + 1 < self.entries.len() { "," } else { "" };
            let _ = writeln!(
                out,
                "    {{\"symbol\": \"{}\", \"research_va\": \"0x{:016x}\", \"release_va\": \"0x{:016x}\", \"status\": \"{}\", \"hits\": {}, \"window\": {}}}{comma}",
                json_escape(&e.symbol),
                e.research_va,
                e.release_va,
                e.status,
                e.hit_count,
                e.window_words
            );
        }
        out.push_str("  ]\n}\n");
        out
    }

    /// Serializes uniquely resolved entries as packed `IvgFwRelocEntry` bytes for `InfernoVgpu.fw`.
    pub fn to_binary(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(self.unique_count * IVG_FW_RELOC_ENTRY_SIZE);
        for e in &self.entries {
            if e.status != "unique" || e.release_va == 0 {
                continue;
            }
            let sym_bytes = e.symbol.as_bytes();
            let mut sym_buf = [0u8; 112];
            if sym_bytes.len() >= sym_buf.len() {
                return Err(format!("symbol '{}' exceeds 111 bytes", e.symbol));
            }
            sym_buf[..sym_bytes.len()].copy_from_slice(sym_bytes);
            let rec = IvgFwRelocEntry {
                target_va: e.release_va,
                kind: 0,
                flags: 1,
                symbol: sym_buf,
            };
            out.extend_from_slice(rec.as_bytes());
        }
        Ok(out)
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::FromBytes;

    #[test]
    fn arm64_masking_normalizes_pc_rel_branches() {
        let bl_a = 0x9400_0100u32;
        let bl_b = 0x9412_3456u32;
        assert_eq!(mask_arm64_insn(bl_a), mask_arm64_insn(bl_b));

        let adrp_x0_a = 0x9000_1200u32;
        let adrp_x0_b = 0xB004_5600u32;
        assert_eq!(mask_arm64_insn(adrp_x0_a), mask_arm64_insn(adrp_x0_b));

        let ret = 0xD65F_03C0u32;
        assert_eq!(mask_arm64_insn(ret), ret);
    }

    #[test]
    fn reloc_binary_roundtrip() {
        let map = ReleaseRelocMap {
            entries: vec![
                ReleaseRelocMatch {
                    symbol: "_IOLog".to_owned(),
                    research_va: 0xffff_fff0_07ff_1b68,
                    release_va: 0xffff_fff0_07ff_1b68,
                    status: "unique".to_owned(),
                    hit_count: 1,
                    window_words: 12,
                },
                ReleaseRelocMatch {
                    symbol: "_MissingSym".to_owned(),
                    research_va: 0,
                    release_va: 0,
                    status: "missing".to_owned(),
                    hit_count: 0,
                    window_words: 0,
                },
            ],
            unique_count: 1,
        };
        let bin = map.to_binary().unwrap();
        assert_eq!(bin.len(), IVG_FW_RELOC_ENTRY_SIZE);
        let parsed = IvgFwRelocEntry::read_from_bytes(&bin).unwrap();
        assert_eq!(parsed.target_va, 0xffff_fff0_07ff_1b68);
        assert_eq!(&parsed.symbol[..6], b"_IOLog");
    }
}
