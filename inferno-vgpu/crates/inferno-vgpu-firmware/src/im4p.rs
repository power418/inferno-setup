//! Minimal Apple `IMG4` / `IM4P` ASN.1 DER extractor and optional `liblzfse` decoder.

use std::ffi::{CStr, CString};
use std::fmt;
use std::path::Path;

/// Error decoding an Apple `IMG4` / `IM4P` payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Im4pError {
    /// Input is not valid DER or missing IM4P elements.
    InvalidDer(&'static str),
    /// Compressed payload requires `liblzfse` which was not found on this host.
    MissingLzfse(String),
    /// LZFSE decompression returned zero bytes or failed.
    LzfseFailed,
}

impl fmt::Display for Im4pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDer(msg) => write!(f, "invalid IMG4/IM4P DER: {msg}"),
            Self::MissingLzfse(msg) => write!(f, "LZFSE decoder unavailable: {msg}"),
            Self::LzfseFailed => write!(f, "LZFSE decompression failed"),
        }
    }
}

impl std::error::Error for Im4pError {}

/// Extracted IM4P payload metadata and raw (possibly compressed) bytes.
#[derive(Clone, Debug)]
pub struct Im4pPayload {
    /// Four-character IM4P type tag (e.g. `"krnl"`, `"rkrn"`, `"dtre"`).
    pub tag: String,
    /// IM4P description string.
    pub description: String,
    /// Raw payload bytes (may start with `"bvx2"` LZFSE or `0xFEEDFACF` Mach-O).
    pub data: Vec<u8>,
}

fn read_der_tlv(data: &[u8], at: usize) -> Result<(u8, usize, usize), Im4pError> {
    let tag = *data.get(at).ok_or(Im4pError::InvalidDer("missing tag"))?;
    let len_byte = *data
        .get(at + 1)
        .ok_or(Im4pError::InvalidDer("missing length"))?;
    let (body, len) = if (len_byte & 0x80) == 0 {
        (at + 2, len_byte as usize)
    } else {
        let count = (len_byte & 0x7F) as usize;
        if count == 0 || count > 8 {
            return Err(Im4pError::InvalidDer("bad definite length"));
        }
        let mut acc = 0usize;
        for i in 0..count {
            let b = *data
                .get(at + 2 + i)
                .ok_or(Im4pError::InvalidDer("truncated length bytes"))?;
            acc = (acc << 8) | (b as usize);
        }
        (at + 2 + count, acc)
    };
    if body.checked_add(len).is_none_or(|e| e > data.len()) {
        return Err(Im4pError::InvalidDer("TLV extends past EOF"));
    }
    Ok((tag, body, len))
}

fn der_children(data: &[u8], start: usize, len: usize) -> Result<Vec<(u8, usize, usize)>, Im4pError> {
    let end = start + len;
    let mut out = Vec::new();
    let mut cur = start;
    while cur < end {
        let (tag, body, n) = read_der_tlv(data, cur)?;
        out.push((tag, body, n));
        cur = body + n;
    }
    Ok(out)
}

/// Extracts the payload from an `IMG4` or `IM4P` DER container.
pub fn extract_im4p(data: &[u8]) -> Result<Im4pPayload, Im4pError> {
    let (tag0, body0, len0) = read_der_tlv(data, 0)?;
    if tag0 != 0x30 {
        return Err(Im4pError::InvalidDer("root element is not SEQUENCE"));
    }
    let mut kids = der_children(data, body0, len0)?;
    let first = kids
        .first()
        .ok_or(Im4pError::InvalidDer("empty root SEQUENCE"))?;
    if &data[first.1..first.1 + first.2] == b"IMG4" {
        let inner = kids
            .get(1)
            .ok_or(Im4pError::InvalidDer("IMG4 missing IM4P child"))?;
        kids = der_children(data, inner.1, inner.2)?;
    }
    if kids.len() < 4 {
        return Err(Im4pError::InvalidDer("IM4P requires at least 4 elements"));
    }
    let magic = &data[kids[0].1..kids[0].1 + kids[0].2];
    if magic != b"IM4P" {
        return Err(Im4pError::InvalidDer("missing IM4P magic"));
    }
    let tag = String::from_utf8_lossy(&data[kids[1].1..kids[1].1 + kids[1].2]).into_owned();
    let description = String::from_utf8_lossy(&data[kids[2].1..kids[2].1 + kids[2].2]).into_owned();
    let payload = data[kids[3].1..kids[3].1 + kids[3].2].to_vec();
    Ok(Im4pPayload {
        tag,
        description,
        data: payload,
    })
}

/// Unwraps raw Mach-O bytes from either a plain Mach-O buffer or an `IMG4`/`IM4P`
/// (transparently decoding `bvx2` LZFSE via `liblzfse.so` if available).
pub fn decode_kernelcache_bytes(
    input: &[u8],
    extra_lzfse_dir: Option<&Path>,
) -> Result<Vec<u8>, Im4pError> {
    if input.len() >= 4 && &input[..4] == &crate::macho::MH_MAGIC_64.to_le_bytes() {
        return Ok(input.to_vec());
    }
    let im4p = extract_im4p(input)?;
    if im4p.data.len() >= 4 && &im4p.data[..4] == &crate::macho::MH_MAGIC_64.to_le_bytes() {
        return Ok(im4p.data);
    }
    if im4p.data.len() >= 4 && &im4p.data[..3] == b"bvx" {
        return decode_lzfse(&im4p.data, extra_lzfse_dir);
    }
    Ok(im4p.data)
}

type LzfseDecodeFn = unsafe extern "C" fn(
    dst_buffer: *mut u8,
    dst_size: usize,
    src_buffer: *const u8,
    src_size: usize,
    scratch: *mut libc::c_void,
) -> usize;

fn decode_lzfse(compressed: &[u8], extra_dir: Option<&Path>) -> Result<Vec<u8>, Im4pError> {
    let mut candidates = Vec::new();
    if let Some(dir) = extra_dir {
        candidates.push(dir.join("liblzfse.so").to_string_lossy().into_owned());
    }
    if let Ok(env_path) = std::env::var("INFERNO_LZFSE_LIB") {
        candidates.push(env_path);
    }
    candidates.push("liblzfse.so".to_owned());
    candidates.push("liblzfse.so.1".to_owned());

    let mut handle: *mut libc::c_void = std::ptr::null_mut();
    for cand in &candidates {
        let Ok(cstr) = CString::new(cand.as_str()) else {
            continue;
        };
        // SAFETY: `cstr` is a valid NUL-terminated library path.
        let h = unsafe { libc::dlopen(cstr.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !h.is_null() {
            handle = h;
            break;
        }
    }
    if handle.is_null() {
        return Err(Im4pError::MissingLzfse(
            "install liblzfse or set INFERNO_LZFSE_LIB=/path/to/liblzfse.so".to_owned(),
        ));
    }

    let sym_name = CStr::from_bytes_with_nul(b"lzfse_decode_buffer\0")
        .map_err(|_| Im4pError::LzfseFailed)?;
    // SAFETY: `handle` is valid and non-null.
    let sym = unsafe { libc::dlsym(handle, sym_name.as_ptr()) };
    if sym.is_null() {
        // SAFETY: closing opened library handle.
        unsafe { libc::dlclose(handle) };
        return Err(Im4pError::MissingLzfse(
            "symbol lzfse_decode_buffer not found in liblzfse".to_owned(),
        ));
    }
    // SAFETY: `lzfse_decode_buffer` has the standard Apple LZFSE C ABI.
    let decode_fn: LzfseDecodeFn = unsafe { std::mem::transmute(sym) };

    let mut cap = compressed.len().saturating_mul(6).max(1024 * 1024);
    for _ in 0..3 {
        let mut out = vec![0u8; cap];
        // SAFETY: valid input/output slices passed to `lzfse_decode_buffer`.
        let n = unsafe {
            decode_fn(
                out.as_mut_ptr(),
                out.len(),
                compressed.as_ptr(),
                compressed.len(),
                std::ptr::null_mut(),
            )
        };
        if n > 0 && n < cap {
            out.truncate(n);
            // SAFETY: closing opened library handle.
            unsafe { libc::dlclose(handle) };
            return Ok(out);
        }
        cap = cap.saturating_mul(2);
    }

    // SAFETY: closing opened library handle.
    unsafe { libc::dlclose(handle) };
    Err(Im4pError::LzfseFailed)
}
