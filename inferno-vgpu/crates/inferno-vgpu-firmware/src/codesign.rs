//! Apple Mach-O ad-hoc `LC_CODE_SIGNATURE` signing, SHA-256 CDHash extraction,
//! and XNU Static TrustCache v1 merging/sorting.

use crate::macho::{LC_CODE_SIGNATURE, LC_SEGMENT_64, MH_MAGIC_64, ParsedMacho};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const CS_MAGIC_EMBEDDED_SIGNATURE: u32 = 0xFADE_0CC0;
const CS_MAGIC_CODEDIRECTORY: u32 = 0xFADE_0C02;
const CS_ADHOC: u32 = 0x0000_0002;
const CS_VERSION: u32 = 0x0002_0400;
const CS_HASH_TYPE_SHA256: u8 = 2;
const CS_PAGE_SHIFT: u8 = 12; /* 4096-byte code directory page slots */
const CS_PAGE_SIZE: usize = 1 << CS_PAGE_SHIFT;
const TC_MAGIC_V1: u32 = 1;
const TC_V1_HEADER_SIZE: usize = 24;
const TC_V1_ENTRY_SIZE: usize = 22;

fn be32(slice: &[u8], off: usize) -> Result<u32, String> {
    let b = slice
        .get(off..off + 4)
        .ok_or_else(|| "truncated big-endian u32".to_owned())?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Parses a 40-character hexadecimal SHA-256 CDHash string into 20 bytes.
pub fn parse_cdhash_hex(hex: &str) -> Result<[u8; 20], String> {
    let clean = hex.trim();
    if clean.len() != 40 {
        return Err(format!("expected 40 hex chars for CDHash, got {}", clean.len()));
    }
    let mut out = [0u8; 20];
    for (idx, chunk) in clean.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| "non-UTF8 CDHash".to_owned())?;
        out[idx] = u8::from_str_radix(s, 16).map_err(|_| format!("invalid hex byte '{s}'"))?;
    }
    Ok(out)
}

/// Formats a 20-byte CDHash as lowercase hex.
pub fn format_cdhash_hex(hash: &[u8; 20]) -> String {
    let mut s = String::with_capacity(40);
    for &b in hash {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Extracts the 20-byte SHA-256 CDHash from a signed 64-bit Mach-O's `LC_CODE_SIGNATURE`.
pub fn extract_cdhash(macho: &[u8]) -> Result<[u8; 20], String> {
    let parsed = ParsedMacho::parse(macho).map_err(|e| format!("parse Mach-O: {e}"))?;
    let Some((dataoff, datasize)) = parsed.code_signature else {
        return Err("Mach-O has no LC_CODE_SIGNATURE".to_owned());
    };
    let start = dataoff as usize;
    let end = start
        .checked_add(datasize as usize)
        .ok_or_else(|| "LC_CODE_SIGNATURE range overflow".to_owned())?;
    let sig = macho
        .get(start..end)
        .ok_or_else(|| "LC_CODE_SIGNATURE out of file bounds".to_owned())?;

    if be32(sig, 0)? != CS_MAGIC_EMBEDDED_SIGNATURE {
        return Err("invalid SuperBlob magic (expected 0xFADE0CC0)".to_owned());
    }
    let count = be32(sig, 8)? as usize;
    let mut chosen: Option<&[u8]> = None;

    for i in 0..count {
        let idx_off = 12usize
            .checked_add(i.saturating_mul(8))
            .ok_or_else(|| "SuperBlob index overflow".to_owned())?;
        let blob_off = be32(sig, idx_off + 4)? as usize;
        if blob_off + 40 > sig.len() {
            continue;
        }
        if be32(sig, blob_off)? != CS_MAGIC_CODEDIRECTORY {
            continue;
        }
        let cd_len = be32(sig, blob_off + 4)? as usize;
        let Some(cd_slice) = sig.get(blob_off..blob_off.saturating_add(cd_len)) else {
            continue;
        };
        let hash_type = cd_slice[37];
        if hash_type == CS_HASH_TYPE_SHA256 {
            chosen = Some(cd_slice);
            break;
        }
        if chosen.is_none() {
            chosen = Some(cd_slice);
        }
    }

    let cd = chosen.ok_or_else(|| "no CodeDirectory blob inside LC_CODE_SIGNATURE".to_owned())?;
    let digest = Sha256::digest(cd);
    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[..20]);
    Ok(out)
}

/// Applies an ad-hoc SHA-256 `LC_CODE_SIGNATURE` to a 64-bit Mach-O and returns `(signed_bytes, cdhash)`.
pub fn adhoc_sign_macho(macho: &[u8], identifier: &str) -> Result<(Vec<u8>, [u8; 20]), String> {
    let parsed = ParsedMacho::parse(macho).map_err(|e| format!("parse Mach-O: {e}"))?;
    if parsed.header.ncmds == 0 {
        return Err("Mach-O has no load commands".to_owned());
    }

    // Locate first non-zero file offset occupied by sections so we verify header padding exists if needed.
    let mut first_sect_off = usize::MAX;
    let mut linkedit_cmd_off: Option<usize> = None;
    let mut linkedit_vmaddr = 0u64;
    let mut linkedit_fileoff = 0u64;
    let mut existing_sig_cmd_off: Option<usize> = None;

    let mut out = macho.to_vec();
    let mut cursor = 32usize;
    let cmds_end = 32 + parsed.header.sizeofcmds as usize;

    for _ in 0..parsed.header.ncmds {
        let cmd = u32::from_le_bytes(
            out.get(cursor..cursor + 4)
                .ok_or_else(|| "truncated cmd".to_owned())?
                .try_into()
                .map_err(|_| "slice".to_owned())?,
        );
        let cmdsize = u32::from_le_bytes(
            out.get(cursor + 4..cursor + 8)
                .ok_or_else(|| "truncated cmdsize".to_owned())?
                .try_into()
                .map_err(|_| "slice".to_owned())?,
        ) as usize;
        if cmd == LC_SEGMENT_64 && cmdsize >= 72 {
            let name_bytes = &out[cursor + 8..cursor + 24];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(16);
            let segname = std::str::from_utf8(&name_bytes[..name_len]).unwrap_or("");
            if segname == "__LINKEDIT" {
                linkedit_cmd_off = Some(cursor);
                linkedit_vmaddr = u64::from_le_bytes(
                    out[cursor + 24..cursor + 32]
                        .try_into()
                        .map_err(|_| "vmaddr".to_owned())?,
                );
                linkedit_fileoff = u64::from_le_bytes(
                    out[cursor + 40..cursor + 48]
                        .try_into()
                        .map_err(|_| "fileoff".to_owned())?,
                );
            }
            let nsects = u32::from_le_bytes(
                out[cursor + 64..cursor + 68]
                    .try_into()
                    .map_err(|_| "nsects".to_owned())?,
            ) as usize;
            for s in 0..nsects {
                let soff = cursor + 72 + s * 80;
                if soff + 52 <= cursor + cmdsize {
                    let sect_off = u32::from_le_bytes(
                        out[soff + 48..soff + 52]
                            .try_into()
                            .map_err(|_| "sectoff".to_owned())?,
                    ) as usize;
                    let sect_size = u64::from_le_bytes(
                        out[soff + 40..soff + 48]
                            .try_into()
                            .map_err(|_| "sectsize".to_owned())?,
                    );
                    if sect_off != 0 && sect_size != 0 {
                        first_sect_off = first_sect_off.min(sect_off);
                    }
                }
            }
        } else if cmd == LC_CODE_SIGNATURE {
            existing_sig_cmd_off = Some(cursor);
        }
        cursor += cmdsize;
    }

    // Strip previous trailing signature bytes if re-signing.
    let base_len = if let Some((old_off, old_size)) = parsed.code_signature
        && (old_off as usize).saturating_add(old_size as usize) <= out.len()
    {
        old_off as usize
    } else {
        out.len()
    };
    out.truncate(base_len);
    let code_limit = (out.len() + 15) & !15;
    out.resize(code_limit, 0);

    // Ensure LC_CODE_SIGNATURE load command fits in header padding.
    let sig_cmd_off = if let Some(off) = existing_sig_cmd_off {
        off
    } else {
        if cmds_end + 16 > first_sect_off || cmds_end + 16 > code_limit {
            return Err(format!(
                "insufficient Mach-O header padding for LC_CODE_SIGNATURE (cmds_end={cmds_end}, first_sect_off={first_sect_off})"
            ));
        }
        let new_ncmds = parsed.header.ncmds + 1;
        let new_sizeofcmds = parsed.header.sizeofcmds + 16;
        out[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        out[16..20].copy_from_slice(&new_ncmds.to_le_bytes());
        out[20..24].copy_from_slice(&new_sizeofcmds.to_le_bytes());
        cmds_end
    };

    // Build CodeDirectory v0x20400 (minimum needed for SHA-256 ad-hoc + execSegBase=0).
    let ident_bytes = identifier.as_bytes();
    let cd_hdr_len = 88usize; /* through execSegFlags (v0x20400) */
    let ident_off = cd_hdr_len;
    let ident_len = ident_bytes.len() + 1;
    let hash_off = (ident_off + ident_len + 3) & !3;
    let n_code_slots = code_limit.div_ceil(CS_PAGE_SIZE);
    let cd_len = hash_off + n_code_slots * 32;

    let mut cd = vec![0u8; cd_len];
    cd[0..4].copy_from_slice(&CS_MAGIC_CODEDIRECTORY.to_be_bytes());
    cd[4..8].copy_from_slice(&(cd_len as u32).to_be_bytes());
    cd[8..12].copy_from_slice(&CS_VERSION.to_be_bytes());
    cd[12..16].copy_from_slice(&CS_ADHOC.to_be_bytes());
    cd[16..20].copy_from_slice(&(hash_off as u32).to_be_bytes());
    cd[20..24].copy_from_slice(&(ident_off as u32).to_be_bytes());
    cd[24..28].copy_from_slice(&0u32.to_be_bytes()); /* nSpecialSlots */
    cd[28..32].copy_from_slice(&(n_code_slots as u32).to_be_bytes());
    cd[32..36].copy_from_slice(&(code_limit as u32).to_be_bytes());
    cd[36] = 32; /* hashSize */
    cd[37] = CS_HASH_TYPE_SHA256;
    cd[38] = 0; /* platform */
    cd[39] = CS_PAGE_SHIFT;
    cd[40..44].copy_from_slice(&0u32.to_be_bytes()); /* spare2 */
    cd[56..64].copy_from_slice(&(code_limit as u64).to_be_bytes()); /* codeLimit64 */
    cd[ident_off..ident_off + ident_bytes.len()].copy_from_slice(ident_bytes);

    for slot in 0..n_code_slots {
        let p_start = slot * CS_PAGE_SIZE;
        let p_end = (p_start + CS_PAGE_SIZE).min(code_limit);
        let page_digest = Sha256::digest(&out[p_start..p_end]);
        let dst = hash_off + slot * 32;
        cd[dst..dst + 32].copy_from_slice(&page_digest);
    }

    let super_hdr_len = 20usize; /* 12B SuperBlob + 1 * 8B BlobIndex */
    let super_len = super_hdr_len + cd.len();
    let mut super_blob = vec![0u8; super_len];
    super_blob[0..4].copy_from_slice(&CS_MAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
    super_blob[4..8].copy_from_slice(&(super_len as u32).to_be_bytes());
    super_blob[8..12].copy_from_slice(&1u32.to_be_bytes()); /* count */
    super_blob[12..16].copy_from_slice(&0u32.to_be_bytes()); /* CSSLOT_CODEDIRECTORY */
    super_blob[16..20].copy_from_slice(&(super_hdr_len as u32).to_be_bytes());
    super_blob[super_hdr_len..].copy_from_slice(&cd);

    let sig_data_off = code_limit as u32;
    let sig_data_size = super_blob.len() as u32;
    out.extend_from_slice(&super_blob);

    // Patch LC_CODE_SIGNATURE command.
    out[sig_cmd_off..sig_cmd_off + 4].copy_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
    out[sig_cmd_off + 4..sig_cmd_off + 8].copy_from_slice(&16u32.to_le_bytes());
    out[sig_cmd_off + 8..sig_cmd_off + 12].copy_from_slice(&sig_data_off.to_le_bytes());
    out[sig_cmd_off + 12..sig_cmd_off + 16].copy_from_slice(&sig_data_size.to_le_bytes());

    // Update __LINKEDIT filesize/vmsize if present so file length remains valid.
    if let Some(le_off) = linkedit_cmd_off {
        let new_filesize = (out.len() as u64).saturating_sub(linkedit_fileoff);
        let new_vmsize = (new_filesize + 0x3FFF) & !0x3FFF;
        if linkedit_vmaddr != 0 || new_filesize > 0 {
            out[le_off + 32..le_off + 40].copy_from_slice(&new_vmsize.to_le_bytes());
            out[le_off + 48..le_off + 56].copy_from_slice(&new_filesize.to_le_bytes());
        }
    }

    let cdhash = extract_cdhash(&out)?;
    Ok((out, cdhash))
}

/// Merges additional 20-byte CDHashes into a TrustCache v1 module payload, sorted & deduplicated.
pub fn merge_trustcache_v1(module: &[u8], extra: &[[u8; 20]]) -> Result<Vec<u8>, String> {
    if module.len() < TC_V1_HEADER_SIZE {
        return Err("TrustCache module shorter than 24-byte header".to_owned());
    }
    let version = u32::from_le_bytes([module[0], module[1], module[2], module[3]]);
    if version != TC_MAGIC_V1 {
        return Err(format!("expected TrustCache v1, got v{version}"));
    }
    let count = u32::from_le_bytes([module[20], module[21], module[22], module[23]]) as usize;
    let expected = TC_V1_HEADER_SIZE + count * TC_V1_ENTRY_SIZE;
    if module.len() != expected {
        return Err(format!(
            "TrustCache v1 length {} != expected {expected} for {count} entries",
            module.len()
        ));
    }

    let mut set: BTreeSet<[u8; 20]> = BTreeSet::new();
    let mut extras_meta: std::collections::BTreeMap<[u8; 20], [u8; 2]> =
        std::collections::BTreeMap::new();
    for i in 0..count {
        let off = TC_V1_HEADER_SIZE + i * TC_V1_ENTRY_SIZE;
        let mut h = [0u8; 20];
        h.copy_from_slice(&module[off..off + 20]);
        set.insert(h);
        extras_meta.insert(h, [module[off + 20], module[off + 21]]);
    }
    for &h in extra {
        set.insert(h);
        extras_meta.entry(h).or_insert([CS_HASH_TYPE_SHA256, 0]);
    }

    let new_count = set.len() as u32;
    let mut out = Vec::with_capacity(TC_V1_HEADER_SIZE + set.len() * TC_V1_ENTRY_SIZE);
    out.extend_from_slice(&TC_MAGIC_V1.to_le_bytes());
    out.extend_from_slice(&module[4..20]); /* preserve original UUID */
    out.extend_from_slice(&new_count.to_le_bytes());
    for h in set {
        out.extend_from_slice(&h);
        let meta = extras_meta.get(&h).copied().unwrap_or([CS_HASH_TYPE_SHA256, 0]);
        out.extend_from_slice(&meta);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macho::{CPU_SUBTYPE_ARM64E, CPU_TYPE_ARM64, LC_SEGMENT_64, MH_DYLIB, MH_MAGIC_64};

    fn synthetic_unsigned_dylib() -> Vec<u8> {
        let seg_size = 72u32 + 80u32;
        let mut le = [0u8; 72];
        le[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        le[4..8].copy_from_slice(&72u32.to_le_bytes());
        le[8..18].copy_from_slice(b"__LINKEDIT");
        le[24..32].copy_from_slice(&0x4000u64.to_le_bytes());
        le[32..40].copy_from_slice(&0x4000u64.to_le_bytes());
        le[40..48].copy_from_slice(&0x1000u64.to_le_bytes());
        le[48..56].copy_from_slice(&0x100u64.to_le_bytes());

        let mut text = vec![0u8; seg_size as usize];
        text[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        text[4..8].copy_from_slice(&seg_size.to_le_bytes());
        text[8..14].copy_from_slice(b"__TEXT");
        text[32..40].copy_from_slice(&0x4000u64.to_le_bytes());
        text[48..56].copy_from_slice(&0x1000u64.to_le_bytes());
        text[64..68].copy_from_slice(&1u32.to_le_bytes());
        text[72..78].copy_from_slice(b"__text");
        text[88..94].copy_from_slice(b"__TEXT");
        text[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        text[112..120].copy_from_slice(&4u64.to_le_bytes());
        text[120..124].copy_from_slice(&0x400u32.to_le_bytes());

        let sizeofcmds = (text.len() + le.len()) as u32;
        let mut out = vec![0u8; 0x1100];
        out[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        out[4..8].copy_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
        out[8..12].copy_from_slice(&CPU_SUBTYPE_ARM64E.to_le_bytes());
        out[12..16].copy_from_slice(&MH_DYLIB.to_le_bytes());
        out[16..20].copy_from_slice(&2u32.to_le_bytes());
        out[20..24].copy_from_slice(&sizeofcmds.to_le_bytes());
        out[32..32 + text.len()].copy_from_slice(&text);
        out[32 + text.len()..32 + sizeofcmds as usize].copy_from_slice(&le);
        /* ret instruction at 0x400 */
        out[0x400..0x404].copy_from_slice(&0xD65F_03C0u32.to_le_bytes());
        out
    }

    #[test]
    fn adhoc_sign_and_extract_cdhash_roundtrip() {
        let dylib = synthetic_unsigned_dylib();
        let (signed, cdhash) = adhoc_sign_macho(&dylib, "dev.inferno.metal.test").unwrap();
        let again = extract_cdhash(&signed).unwrap();
        assert_eq!(cdhash, again);
        assert_ne!(cdhash, [0u8; 20]);
    }

    #[test]
    fn trustcache_v1_merge_sort_dedup() {
        let mut tc = Vec::new();
        tc.extend_from_slice(&1u32.to_le_bytes());
        tc.extend_from_slice(&[0xABu8; 16]);
        tc.extend_from_slice(&2u32.to_le_bytes());
        tc.extend_from_slice(&[0x10u8; 20]);
        tc.extend_from_slice(&[2u8, 0]);
        tc.extend_from_slice(&[0x30u8; 20]);
        tc.extend_from_slice(&[2u8, 0]);

        let merged = merge_trustcache_v1(&tc, &[[0x20u8; 20], [0x10u8; 20]]).unwrap();
        let cnt = u32::from_le_bytes(merged[20..24].try_into().unwrap());
        assert_eq!(cnt, 3);
        assert_eq!(&merged[24..44], &[0x10u8; 20]);
        assert_eq!(&merged[46..66], &[0x20u8; 20]);
        assert_eq!(&merged[68..88], &[0x30u8; 20]);
    }
}
