//! Unix domain socket + `SCM_RIGHTS` framing protocol between Inferno's
//! `apple_pvgpu.c` and the out-of-process `inferno-vgpu-server`.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// IPC message types.
pub mod msg {
    /// Client -> Server: request region FD and display geometry.
    pub const HELLO: u32 = 1;
    /// Server -> Client: returns region_size (`arg1`), `(w<<32)|h` (`arg2`), plus `SCM_RIGHTS` `memfd`.
    pub const HELLO_ACK: u32 = 2;
    /// Client -> Server: 32-bit MMIO read (`arg1 = offset`).
    pub const MMIO_READ: u32 = 3;
    /// Server -> Client: `arg0 = value`.
    pub const MMIO_READ_ACK: u32 = 4;
    /// Client -> Server: 32-bit MMIO write (`arg0 = value`, `arg1 = offset`).
    pub const MMIO_WRITE: u32 = 5;
    /// Server -> Client: generic completion ack.
    pub const ACK: u32 = 6;
    /// Client -> Server: doorbell write (`arg1 = queue_id`), fire-and-forget (no ACK).
    pub const DOORBELL: u32 = 7;
    /// Client -> Server: reset device state.
    pub const RESET: u32 = 8;
    /// Client -> Server: read latest BGRA8 frame into trailing payload (`arg1 = max_bytes`).
    pub const READ_DISPLAY: u32 = 9;
    /// Server -> Client: `arg0 = width`, `arg1 = height`, `arg2 = payload_len`, followed by BGRA8 bytes.
    pub const READ_DISPLAY_ACK: u32 = 10;
    /// Server -> Client (async): raise interrupt bits (`arg0 = irq_bits`).
    pub const IRQ_RAISE: u32 = 11;
    /// Client -> Server: request host GPU `IvgDeviceCaps` struct.
    pub const GET_CAPS: u32 = 12;
    /// Server -> Client: `arg2 = sizeof(IvgDeviceCaps)`, followed by `IvgDeviceCaps` bytes.
    pub const GET_CAPS_ACK: u32 = 13;
}

/// Fixed 24-byte wire packet header over the Unix socket.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromBytes, IntoBytes, Immutable, KnownLayout)]
pub struct IpcHeader {
    /// Message type (`msg::*`).
    pub msg_type: u32,
    /// 32-bit argument / value.
    pub arg0: u32,
    /// 64-bit argument (offset / queue_id / size).
    pub arg1: u64,
    /// Secondary 64-bit argument (geometry / payload byte length).
    pub arg2: u64,
}
