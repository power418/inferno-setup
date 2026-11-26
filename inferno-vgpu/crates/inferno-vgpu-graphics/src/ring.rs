//! Lockless SPSC shared-memory ring buffer reader & writer.
//!
//! All memory accesses to shared memory use volatile/atomic byte operations on a
//! raw region pointer so there is never an aliased Rust reference into memory
//! concurrently mutated by the guest vCPU.

use crate::wire::{IVG_RING_FLAG_HOST_IDLE, IvgRecordHeader, IvgRingHeader};
use core::mem::size_of;
use core::sync::atomic::{AtomicU32, Ordering};
use zerocopy::{FromBytes, IntoBytes};

/// Error returned when a shared-memory ring header or record is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    /// Ring header offset is out of bounds or unaligned.
    BadHeaderOffset,
    /// Ring capacity is not a valid power-of-two or overflows region.
    BadCapacity,
    /// Record header is truncated or exceeds ring capacity.
    MalformedRecord,
    /// Not enough free space in ring to write record.
    Full,
}

/// View over a ring header & data buffer inside Region 1.
///
/// # Safety invariant
/// `region_base` must point to a valid mapped region of at least `region_len`
/// bytes for the lifetime of operations on `SharedRing`.
#[derive(Clone, Copy)]
pub struct SharedRing {
    region_base: *mut u8,
    region_len: usize,
    header_offset: usize,
}

// SAFETY: SharedRing only accesses shared memory atomically/volatilely under SPSC ordering.
unsafe impl Send for SharedRing {}
// SAFETY: SharedRing only accesses shared memory atomically/volatilely under SPSC ordering.
unsafe impl Sync for SharedRing {}

impl SharedRing {
    /// Validates and constructs a `SharedRing` view at `header_offset` within `region`.
    ///
    /// # Safety
    /// `region_base..region_base + region_len` must remain mapped and valid while
    /// this `SharedRing` is used.
    pub unsafe fn new(
        region_base: *mut u8,
        region_len: usize,
        header_offset: usize,
    ) -> Result<Self, RingError> {
        if region_base.is_null() || !header_offset.is_multiple_of(64) {
            return Err(RingError::BadHeaderOffset);
        }
        let end = header_offset
            .checked_add(size_of::<IvgRingHeader>())
            .ok_or(RingError::BadHeaderOffset)?;
        if end > region_len {
            return Err(RingError::BadHeaderOffset);
        }
        let ring = Self {
            region_base,
            region_len,
            header_offset,
        };
        let _ = ring.validate_geometry()?;
        Ok(ring)
    }

    /// Initializes a ring header in shared memory at `header_offset` with data
    /// buffer at `data_offset` of `capacity_bytes` (must be power of 2 >= 64).
    ///
    /// # Safety
    /// Caller must ensure exclusive initialization access to `[header_offset..]`
    /// and `[data_offset..data_offset + capacity_bytes]` inside valid `region_base`.
    pub unsafe fn init(
        region_base: *mut u8,
        region_len: usize,
        header_offset: usize,
        data_offset: usize,
        capacity_bytes: u32,
    ) -> Result<Self, RingError> {
        if region_base.is_null() || !header_offset.is_multiple_of(64) {
            return Err(RingError::BadHeaderOffset);
        }
        if !capacity_bytes.is_power_of_two() || capacity_bytes < 64 {
            return Err(RingError::BadCapacity);
        }
        let hdr_end = header_offset
            .checked_add(size_of::<IvgRingHeader>())
            .ok_or(RingError::BadHeaderOffset)?;
        let data_end = data_offset
            .checked_add(capacity_bytes as usize)
            .ok_or(RingError::BadCapacity)?;
        if hdr_end > region_len || data_end > region_len {
            return Err(RingError::BadCapacity);
        }
        let hdr = IvgRingHeader {
            producer: 0,
            consumer: 0,
            capacity_mask: capacity_bytes - 1,
            flags: IVG_RING_FLAG_HOST_IDLE,
            data_offset: data_offset as u64,
            reserved: [0; 5],
        };
        // SAFETY: Checked bounds and alignment above.
        unsafe {
            let dst = region_base.add(header_offset);
            core::ptr::copy_nonoverlapping(hdr.as_bytes().as_ptr(), dst, size_of::<IvgRingHeader>());
        }
        // SAFETY: Validated geometry.
        unsafe { Self::new(region_base, region_len, header_offset) }
    }

    fn header_ptr(&self) -> *mut IvgRingHeader {
        // SAFETY: `header_offset + size_of::<IvgRingHeader>() <= region_len` checked in `new`.
        unsafe { self.region_base.add(self.header_offset).cast::<IvgRingHeader>() }
    }

    fn field_atomic(&self, byte_offset: usize) -> &AtomicU32 {
        // SAFETY: `header_offset` is 64-byte aligned, fields are 4-byte aligned within `IvgRingHeader`.
        unsafe { &*(self.region_base.add(self.header_offset + byte_offset).cast::<AtomicU32>()) }
    }

    fn producer_atomic(&self) -> &AtomicU32 {
        self.field_atomic(0)
    }

    fn consumer_atomic(&self) -> &AtomicU32 {
        self.field_atomic(4)
    }

    fn flags_atomic(&self) -> &AtomicU32 {
        self.field_atomic(12)
    }

    /// Reads and validates `(data_offset, capacity, mask)` from shared header.
    pub fn validate_geometry(&self) -> Result<(usize, u32, u32), RingError> {
        // SAFETY: Volatile reads avoid UB with concurrent guest access; capacity/data_offset are static.
        let (mask, data_offset) = unsafe {
            let hdr = self.header_ptr();
            let mask = core::ptr::read_volatile(&(*hdr).capacity_mask);
            let data_off = core::ptr::read_volatile(&(*hdr).data_offset);
            (mask, data_off)
        };
        let cap = mask.checked_add(1).ok_or(RingError::BadCapacity)?;
        if !cap.is_power_of_two() || cap < 64 {
            return Err(RingError::BadCapacity);
        }
        let data_off = usize::try_from(data_offset).map_err(|_| RingError::BadCapacity)?;
        let end = data_off
            .checked_add(cap as usize)
            .ok_or(RingError::BadCapacity)?;
        if end > self.region_len {
            return Err(RingError::BadCapacity);
        }
        Ok((data_off, cap, mask))
    }

    /// Sets or clears `IVG_RING_FLAG_HOST_IDLE` atomically.
    pub fn set_host_idle(&self, idle: bool) {
        if idle {
            self.flags_atomic()
                .fetch_or(IVG_RING_FLAG_HOST_IDLE, Ordering::Release);
        } else {
            self.flags_atomic()
                .fetch_and(!IVG_RING_FLAG_HOST_IDLE, Ordering::Release);
        }
    }

    /// Returns current ring flags.
    pub fn flags(&self) -> u32 {
        self.flags_atomic().load(Ordering::Acquire)
    }

    /// Attempts to pop the next complete record from the ring into `scratch`.
    /// Returns `Ok(Some(len))` when a record was read, `Ok(None)` when empty.
    pub fn try_pop_record(&self, scratch: &mut Vec<u8>) -> Result<Option<usize>, RingError> {
        let (data_off, cap, mask) = self.validate_geometry()?;
        let prod = self.producer_atomic().load(Ordering::Acquire);
        let cons = self.consumer_atomic().load(Ordering::Relaxed);
        let avail = prod.wrapping_sub(cons);
        if avail == 0 {
            return Ok(None);
        }
        if avail > cap || avail < size_of::<IvgRecordHeader>() as u32 {
            return Err(RingError::MalformedRecord);
        }

        let mut hdr_bytes = [0u8; size_of::<IvgRecordHeader>()];
        self.read_ring_bytes(data_off, cap, mask, cons, &mut hdr_bytes);
        let hdr = IvgRecordHeader::read_from_bytes(&hdr_bytes)
            .map_err(|_| RingError::MalformedRecord)?;

        let rec_size = hdr.size;
        if rec_size < size_of::<IvgRecordHeader>() as u32 || rec_size > avail {
            return Err(RingError::MalformedRecord);
        }
        let aligned_size = align8(rec_size).ok_or(RingError::MalformedRecord)?;
        if aligned_size > avail {
            return Err(RingError::MalformedRecord);
        }

        scratch.resize(rec_size as usize, 0);
        self.read_ring_bytes(data_off, cap, mask, cons, scratch);

        self.consumer_atomic()
            .store(cons.wrapping_add(aligned_size), Ordering::Release);
        Ok(Some(rec_size as usize))
    }

    /// Attempts to pop a fixed-size record (such as `IvgCompletionRecord`) into `dst`.
    /// Returns `Ok(true)` when filled, `Ok(false)` when ring is empty.
    pub fn try_pop_fixed(&self, dst: &mut [u8]) -> Result<bool, RingError> {
        let (data_off, cap, mask) = self.validate_geometry()?;
        let len_u32 = u32::try_from(dst.len()).map_err(|_| RingError::MalformedRecord)?;
        let aligned = align8(len_u32).ok_or(RingError::MalformedRecord)?;
        let prod = self.producer_atomic().load(Ordering::Acquire);
        let cons = self.consumer_atomic().load(Ordering::Relaxed);
        let avail = prod.wrapping_sub(cons);
        if avail == 0 {
            return Ok(false);
        }
        if avail > cap || aligned > avail || aligned == 0 {
            return Err(RingError::MalformedRecord);
        }
        self.read_ring_bytes(data_off, cap, mask, cons, dst);
        self.consumer_atomic()
            .store(cons.wrapping_add(aligned), Ordering::Release);
        Ok(true)
    }

    /// Pushes a complete record (or arbitrary aligned payload) into the ring.
    pub fn push_bytes(&self, bytes: &[u8]) -> Result<(), RingError> {
        let (data_off, cap, mask) = self.validate_geometry()?;
        let len_u32 = u32::try_from(bytes.len()).map_err(|_| RingError::Full)?;
        let aligned = align8(len_u32).ok_or(RingError::Full)?;
        if aligned > cap {
            return Err(RingError::Full);
        }

        let cons = self.consumer_atomic().load(Ordering::Acquire);
        let prod = self.producer_atomic().load(Ordering::Relaxed);
        let used = prod.wrapping_sub(cons);
        if used > cap || cap - used < aligned {
            return Err(RingError::Full);
        }

        self.write_ring_bytes(data_off, cap, mask, prod, bytes);
        if aligned > len_u32 {
            let pad = [0u8; 8];
            let pad_len = (aligned - len_u32) as usize;
            self.write_ring_bytes(
                data_off,
                cap,
                mask,
                prod.wrapping_add(len_u32),
                &pad[..pad_len],
            );
        }
        self.producer_atomic()
            .store(prod.wrapping_add(aligned), Ordering::Release);
        Ok(())
    }

    /// Reads `dst.len()` bytes starting at ring byte cursor `cursor` (handling wrap-around).
    fn read_ring_bytes(&self, data_off: usize, cap: u32, mask: u32, cursor: u32, dst: &mut [u8]) {
        let start = (cursor & mask) as usize;
        let cap_usize = cap as usize;
        let first = dst.len().min(cap_usize - start);
        // SAFETY: `data_off + cap_usize <= region_len` checked in `validate_geometry`.
        unsafe {
            let src = self.region_base.add(data_off + start);
            for (i, b) in dst[..first].iter_mut().enumerate() {
                *b = core::ptr::read_volatile(src.add(i));
            }
            if first < dst.len() {
                let src2 = self.region_base.add(data_off);
                for (i, b) in dst[first..].iter_mut().enumerate() {
                    *b = core::ptr::read_volatile(src2.add(i));
                }
            }
        }
    }

    /// Writes `src` starting at ring byte cursor `cursor` (handling wrap-around).
    fn write_ring_bytes(&self, data_off: usize, cap: u32, mask: u32, cursor: u32, src: &[u8]) {
        let start = (cursor & mask) as usize;
        let cap_usize = cap as usize;
        let first = src.len().min(cap_usize - start);
        // SAFETY: `data_off + cap_usize <= region_len` checked in `validate_geometry`.
        unsafe {
            let dst = self.region_base.add(data_off + start);
            for (i, &b) in src[..first].iter().enumerate() {
                core::ptr::write_volatile(dst.add(i), b);
            }
            if first < src.len() {
                let dst2 = self.region_base.add(data_off);
                for (i, &b) in src[first..].iter().enumerate() {
                    core::ptr::write_volatile(dst2.add(i), b);
                }
            }
        }
    }
}

const fn align8(n: u32) -> Option<u32> {
    match n.checked_add(7) {
        Some(v) => Some(v & !7),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{IVG_MAGIC, IvgCompletionRecord, control_op};

    #[test]
    fn ring_wraparound_and_roundtrip() {
        let mut mem = vec![0u8; 4096];
        // SAFETY: mem stays alive and exclusively owned in this test.
        let ring = unsafe { SharedRing::init(mem.as_mut_ptr(), mem.len(), 0, 128, 128) }
            .expect("ring init");

        for i in 0..20u64 {
            let rec = IvgRecordHeader {
                opcode: control_op::PING,
                flags: 0,
                size: size_of::<IvgRecordHeader>() as u32 + 8,
                context_id: 1,
                reserved: IVG_MAGIC,
                seqno: i,
            };
            let mut buf = Vec::new();
            buf.extend_from_slice(rec.as_bytes());
            buf.extend_from_slice(&(i * 17).to_le_bytes());
            ring.push_bytes(&buf).expect("push");

            let mut out = Vec::new();
            let got = ring.try_pop_record(&mut out).expect("pop").expect("some");
            assert_eq!(got, buf.len());
            assert_eq!(out, buf);
        }

        let cmp = IvgCompletionRecord {
            seqno: 99,
            status: 0,
            refusal_code: 0,
            gpu_start_ns: 10,
            gpu_end_ns: 20,
            result_u64: 1234,
        };
        ring.push_bytes(cmp.as_bytes()).expect("push completion");
    }

    #[test]
    fn ring_rejects_corrupt_header_without_panic() {
        let mut mem = vec![0u8; 256];
        // SAFETY: mem is valid.
        let res = unsafe { SharedRing::init(mem.as_mut_ptr(), mem.len(), 0, 128, 256) };
        assert_eq!(res.err(), Some(RingError::BadCapacity));

        // Corrupt capacity after valid init and verify validation rejects it cleanly.
        let ring = unsafe { SharedRing::init(mem.as_mut_ptr(), mem.len(), 0, 128, 64) }
            .expect("valid init");
        mem[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut scratch = Vec::new();
        assert_eq!(
            ring.try_pop_record(&mut scratch),
            Err(RingError::BadCapacity)
        );
    }
}
