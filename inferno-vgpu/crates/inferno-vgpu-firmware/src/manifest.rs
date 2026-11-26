//! `InfernoVgpu.fw` (`IVGF`) binary firmware/guest package builder and verifier.
//!
//! Layout is ABI-identical to `firmware/include/inferno_firmware_abi.h`.

use sha2::{Digest, Sha256};
use std::fmt;
use std::mem::size_of;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// Binary magic `"IVGF"` little-endian (`0x46475649`).
pub const IVG_FW_MAGIC: u32 = 0x4647_5649;
/// Current package format version.
pub const IVG_FW_VERSION: u32 = 1;
/// Exact `IvgFwHeader` size in bytes.
pub const IVG_FW_HEADER_SIZE: u32 = 128;
/// Exact `IvgFwEntry` size in bytes.
pub const IVG_FW_ENTRY_SIZE: u32 = 256;
/// Maximum number of entries allowed in a single package.
pub const IVG_FW_MAX_ENTRIES: usize = 16;

/// Header flag: switch `product/graphics-featureset-class` to `featureset_class`.
pub const IVG_FW_FLAG_ENABLE_METAL_FEATURESET: u32 = 1 << 0;
/// Header flag: package carries at least one `IVG_FW_ENTRY_KEXT`.
pub const IVG_FW_FLAG_HAS_KEXT: u32 = 1 << 1;
/// Header flag: package carries at least one `IVG_FW_ENTRY_BUNDLE`.
pub const IVG_FW_FLAG_HAS_BUNDLE: u32 = 1 << 2;

/// Entry kind: iOS kernel extension (`arm64e` Mach-O + optional `Info.plist` XML).
pub const IVG_FW_ENTRY_KEXT: u32 = 1;
/// Entry kind: iOS Metal userspace bundle (`InfernoMetal.bundle`).
pub const IVG_FW_ENTRY_BUNDLE: u32 = 2;
/// Entry kind: Device Tree overlay metadata.
pub const IVG_FW_ENTRY_DT_OVERLAY: u32 = 3;

/// Entry flag: payload is `arm64e`.
pub const IVG_FW_ENTRY_FLAG_ARM64E: u32 = 1 << 0;
/// Entry flag: prelink into kernelcache `__PRELINK_INFO`.
pub const IVG_FW_ENTRY_FLAG_PRELINK: u32 = 1 << 1;
/// Entry flag: entry is mandatory for GPU activation.
pub const IVG_FW_ENTRY_FLAG_REQUIRED: u32 = 1 << 2;

/// Fixed 128-byte binary package header (`ivg_fw_header_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgFwHeader {
    /// Must equal [`IVG_FW_MAGIC`].
    pub magic: u32,
    /// Must equal [`IVG_FW_VERSION`].
    pub version: u32,
    /// Must equal [`IVG_FW_HEADER_SIZE`] (`128`).
    pub header_size: u32,
    /// Bitmask of `IVG_FW_FLAG_*`.
    pub flags: u32,
    /// Total package byte length including header, entries, and payloads.
    pub total_size: u64,
    /// Number of [`IvgFwEntry`] descriptors immediately following this header.
    pub entry_count: u32,
    /// Minimum iOS major version (e.g. `14`).
    pub min_ios_major: u16,
    /// Minimum iOS minor version (e.g. `0`).
    pub min_ios_minor: u16,
    /// SHA-256 of bytes `[header_size .. total_size]`.
    pub payload_sha256: [u8; 32],
    /// Null-terminated ASCII `graphics-featureset-class` (e.g. `"MTL2,2"`).
    pub featureset_class: [u8; 32],
    /// Reserved zero bytes.
    pub reserved: [u8; 32],
}

/// Fixed 256-byte binary entry descriptor (`ivg_fw_entry_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IvgFwEntry {
    /// Entry kind (`IVG_FW_ENTRY_*`).
    pub kind: u32,
    /// Entry flags (`IVG_FW_ENTRY_FLAG_*`).
    pub flags: u32,
    /// Byte offset of entry payload from start of package.
    pub payload_offset: u64,
    /// Byte length of entry payload.
    pub payload_size: u64,
    /// Byte offset of optional XML plist (`0` if none).
    pub xml_offset: u64,
    /// Byte length of optional XML plist (`0` if none).
    pub xml_size: u64,
    /// SHA-256 of payload bytes.
    pub payload_sha256: [u8; 32],
    /// Null-terminated bundle identifier (e.g. `"dev.inferno.driver.InfernoVgpuDriver"`).
    pub identifier: [u8; 64],
    /// Null-terminated guest installation path (e.g. `"/System/Library/Extensions/AGXMetalA13.bundle/AGXMetalA13"`).
    pub target_path: [u8; 120],
}

const _: () = assert!(size_of::<IvgFwHeader>() == IVG_FW_HEADER_SIZE as usize);
const _: () = assert!(size_of::<IvgFwEntry>() == IVG_FW_ENTRY_SIZE as usize);

/// Error produced when encoding or verifying an `InfernoVgpu.fw` package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// Too many entries or oversized field.
    LimitExceeded(&'static str),
    /// Truncated or out-of-bounds package bytes.
    Malformed(&'static str),
    /// Header magic or version mismatch.
    BadMagicOrVersion,
    /// SHA-256 digest mismatch on header or entry payload.
    ChecksumMismatch(String),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded(what) => write!(f, "firmware package limit exceeded: {what}"),
            Self::Malformed(what) => write!(f, "malformed firmware package: {what}"),
            Self::BadMagicOrVersion => write!(f, "bad IVGF magic or unsupported version"),
            Self::ChecksumMismatch(what) => write!(f, "SHA-256 checksum mismatch: {what}"),
        }
    }
}

impl std::error::Error for PackageError {}

/// High-level entry specification for building a package.
#[derive(Clone, Debug)]
pub struct PackageEntrySpec {
    /// Entry kind (`IVG_FW_ENTRY_*`).
    pub kind: u32,
    /// Entry flags (`IVG_FW_ENTRY_FLAG_*`).
    pub flags: u32,
    /// Bundle identifier.
    pub identifier: String,
    /// Target path inside iOS rootfs (if applicable).
    pub target_path: String,
    /// Binary payload bytes.
    pub payload: Vec<u8>,
    /// Optional XML `Info.plist` / `IOKitPersonalities` bytes.
    pub xml: Vec<u8>,
}

/// Verified view over a decoded `InfernoVgpu.fw` package.
#[derive(Clone, Debug)]
pub struct VerifiedPackage {
    /// Verified header.
    pub header: IvgFwHeader,
    /// Verified entries.
    pub entries: Vec<IvgFwEntry>,
}

/// Serializes an `InfernoVgpu.fw` binary package from high-level entry specs.
pub fn build_package(
    min_ios: (u16, u16),
    featureset_class: &str,
    enable_metal_featureset: bool,
    specs: &[PackageEntrySpec],
) -> Result<Vec<u8>, PackageError> {
    if specs.len() > IVG_FW_MAX_ENTRIES {
        return Err(PackageError::LimitExceeded("entry_count > 16"));
    }

    let table_bytes = specs.len() * size_of::<IvgFwEntry>();
    let mut cursor = align16(size_of::<IvgFwHeader>() + table_bytes);

    let mut entries = Vec::with_capacity(specs.len());
    let mut flags = if enable_metal_featureset {
        IVG_FW_FLAG_ENABLE_METAL_FEATURESET
    } else {
        0
    };

    for spec in specs {
        if spec.kind == IVG_FW_ENTRY_KEXT {
            flags |= IVG_FW_FLAG_HAS_KEXT;
        } else if spec.kind == IVG_FW_ENTRY_BUNDLE {
            flags |= IVG_FW_FLAG_HAS_BUNDLE;
        }
        let payload_offset = cursor as u64;
        let payload_size = spec.payload.len() as u64;
        cursor = align16(cursor + spec.payload.len());

        let (xml_offset, xml_size) = if spec.xml.is_empty() {
            (0u64, 0u64)
        } else {
            let off = cursor as u64;
            let len = spec.xml.len() as u64;
            cursor = align16(cursor + spec.xml.len());
            (off, len)
        };

        let digest: [u8; 32] = Sha256::digest(&spec.payload).into();
        let mut identifier = [0u8; 64];
        copy_cstr(&mut identifier, &spec.identifier)?;
        let mut target_path = [0u8; 120];
        copy_cstr(&mut target_path, &spec.target_path)?;

        entries.push(IvgFwEntry {
            kind: spec.kind,
            flags: spec.flags,
            payload_offset,
            payload_size,
            xml_offset,
            xml_size,
            payload_sha256: digest,
            identifier,
            target_path,
        });
    }

    let total_size = cursor;
    let mut out = vec![0u8; total_size];

    let mut table_off = size_of::<IvgFwHeader>();
    for (entry, spec) in entries.iter().zip(specs.iter()) {
        let ebytes = entry.as_bytes();
        out[table_off..table_off + ebytes.len()].copy_from_slice(ebytes);
        table_off += ebytes.len();

        let poff = entry.payload_offset as usize;
        out[poff..poff + spec.payload.len()].copy_from_slice(&spec.payload);
        if !spec.xml.is_empty() {
            let xoff = entry.xml_offset as usize;
            out[xoff..xoff + spec.xml.len()].copy_from_slice(&spec.xml);
        }
    }

    let body_sha: [u8; 32] = Sha256::digest(&out[size_of::<IvgFwHeader>()..]).into();
    let mut fs_class = [0u8; 32];
    copy_cstr(&mut fs_class, featureset_class)?;

    let header = IvgFwHeader {
        magic: IVG_FW_MAGIC,
        version: IVG_FW_VERSION,
        header_size: IVG_FW_HEADER_SIZE,
        flags,
        total_size: total_size as u64,
        entry_count: entries.len() as u32,
        min_ios_major: min_ios.0,
        min_ios_minor: min_ios.1,
        payload_sha256: body_sha,
        featureset_class: fs_class,
        reserved: [0u8; 32],
    };
    out[..size_of::<IvgFwHeader>()].copy_from_slice(header.as_bytes());
    Ok(out)
}

/// Validates package framing, bounds, and SHA-256 digests on both header and entries.
pub fn verify_package(bytes: &[u8]) -> Result<VerifiedPackage, PackageError> {
    if bytes.len() < size_of::<IvgFwHeader>() {
        return Err(PackageError::Malformed("smaller than header"));
    }
    let header = IvgFwHeader::read_from_bytes(&bytes[..size_of::<IvgFwHeader>()])
        .map_err(|_| PackageError::Malformed("unaligned header"))?;
    if header.magic != IVG_FW_MAGIC
        || header.version != IVG_FW_VERSION
        || header.header_size != IVG_FW_HEADER_SIZE
    {
        return Err(PackageError::BadMagicOrVersion);
    }
    if header.total_size as usize != bytes.len() {
        return Err(PackageError::Malformed("total_size does not match file length"));
    }
    let count = header.entry_count as usize;
    if count > IVG_FW_MAX_ENTRIES {
        return Err(PackageError::LimitExceeded("entry_count > 16"));
    }

    let actual_body_sha: [u8; 32] = Sha256::digest(&bytes[size_of::<IvgFwHeader>()..]).into();
    if actual_body_sha != header.payload_sha256 {
        return Err(PackageError::ChecksumMismatch("package body".to_owned()));
    }

    let table_start = size_of::<IvgFwHeader>();
    let table_end = table_start + count * size_of::<IvgFwEntry>();
    if table_end > bytes.len() {
        return Err(PackageError::Malformed("entry table past EOF"));
    }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = table_start + i * size_of::<IvgFwEntry>();
        let entry = IvgFwEntry::read_from_bytes(&bytes[off..off + size_of::<IvgFwEntry>()])
            .map_err(|_| PackageError::Malformed("entry read"))?;

        let poff = usize::try_from(entry.payload_offset)
            .map_err(|_| PackageError::Malformed("payload_offset overflow"))?;
        let plen = usize::try_from(entry.payload_size)
            .map_err(|_| PackageError::Malformed("payload_size overflow"))?;
        let pend = poff
            .checked_add(plen)
            .ok_or(PackageError::Malformed("payload range overflow"))?;
        if pend > bytes.len() {
            return Err(PackageError::Malformed("payload extends past EOF"));
        }
        let actual_entry_sha: [u8; 32] = Sha256::digest(&bytes[poff..pend]).into();
        if actual_entry_sha != entry.payload_sha256 {
            return Err(PackageError::ChecksumMismatch(cstr_to_string(&entry.identifier)));
        }

        if entry.xml_size > 0 {
            let xoff = usize::try_from(entry.xml_offset)
                .map_err(|_| PackageError::Malformed("xml_offset overflow"))?;
            let xlen = usize::try_from(entry.xml_size)
                .map_err(|_| PackageError::Malformed("xml_size overflow"))?;
            if xoff.checked_add(xlen).is_none_or(|e| e > bytes.len()) {
                return Err(PackageError::Malformed("xml extends past EOF"));
            }
        }
        entries.push(entry);
    }

    Ok(VerifiedPackage { header, entries })
}

/// Extracts a UTF-8 string up to the first NUL byte.
pub fn cstr_to_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn copy_cstr(dst: &mut [u8], s: &str) -> Result<(), PackageError> {
    let b = s.as_bytes();
    if b.len() >= dst.len() {
        return Err(PackageError::LimitExceeded("string field too long"));
    }
    dst.fill(0);
    dst[..b.len()].copy_from_slice(b);
    Ok(())
}

const fn align16(v: usize) -> usize {
    (v + 15) & !15
}
