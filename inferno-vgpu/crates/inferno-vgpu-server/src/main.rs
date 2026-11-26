//! Out-of-process GPU acceleration server (`inferno-vgpu-server`) isolating
//! host Vulkan / `metal2vulkan` execution from Inferno/SEP state.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod protocol;

use inferno_vgpu::{DeviceConfig, GraphicsDevice, Platform, RtPlatform};
use parking_lot::Mutex;
use protocol::{IpcHeader, msg};
use std::env;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use zerocopy::{FromBytes, IntoBytes};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    // SAFETY: Standard POSIX async-signal-safe handler writing an `AtomicBool`.
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

struct ServerArgs {
    socket_path: PathBuf,
    region_mb: usize,
    width: u32,
    height: u32,
    use_fake: bool,
}

fn parse_args() -> Result<ServerArgs, String> {
    let default_sock = env::var("GPU_SERVER_SOCKET").unwrap_or_else(|_| {
        let uid = unsafe { libc::getuid() };
        format!("/run/user/{uid}/inferno-gpu.sock")
    });
    let mut args = ServerArgs {
        socket_path: PathBuf::from(default_sock),
        region_mb: 64,
        width: 828,
        height: 1792,
        use_fake: false,
    };

    let mut it = env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--socket" => {
                let val = it.next().ok_or("--socket requires a path")?;
                args.socket_path = PathBuf::from(val);
            }
            "--region-mb" => {
                let val = it.next().ok_or("--region-mb requires a number")?;
                args.region_mb = val.parse().map_err(|_| "invalid --region-mb")?;
            }
            "--width" => {
                let val = it.next().ok_or("--width requires a number")?;
                args.width = val.parse().map_err(|_| "invalid --width")?;
            }
            "--height" => {
                let val = it.next().ok_or("--height requires a number")?;
                args.height = val.parse().map_err(|_| "invalid --height")?;
            }
            "--fake" => {
                args.use_fake = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: inferno-vgpu-server [--socket PATH] [--region-mb N] [--width W] [--height H] [--fake]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn send_fd_packet(stream: &UnixStream, hdr: &IpcHeader, fd: i32) -> io::Result<()> {
    let mut buf = [0u8; std::mem::size_of::<IpcHeader>()];
    buf.copy_from_slice(hdr.as_bytes());
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: buf.len(),
    };
    let mut cmsg_buf = [0u8; unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) } as usize];
    // SAFETY: Zeroed msghdr with valid stack buffers.
    unsafe {
        let mut msg_hdr: libc::msghdr = std::mem::zeroed();
        msg_hdr.msg_iov = &mut iov;
        msg_hdr.msg_iovlen = 1;
        msg_hdr.msg_control = cmsg_buf.as_mut_ptr().cast::<libc::c_void>();
        msg_hdr.msg_controllen = cmsg_buf.len() as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg_hdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as _;
        std::ptr::copy_nonoverlapping(
            (&fd as *const i32).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            std::mem::size_of::<i32>(),
        );
        let n = libc::sendmsg(stream.as_raw_fd(), &msg_hdr, 0);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn serve_connection<P: Platform>(
    mut stream: UnixStream,
    device: &GraphicsDevice<P>,
    writer: &Arc<Mutex<Option<UnixStream>>>,
    width: u32,
    height: u32,
) {
    if let Ok(dup) = stream.try_clone() {
        *writer.lock() = Some(dup);
    }

    let mut frame_buf = Vec::new();
    let mut hdr_buf = [0u8; std::mem::size_of::<IpcHeader>()];

    while !SHUTDOWN.load(Ordering::Acquire) {
        if let Err(e) = stream.read_exact(&mut hdr_buf) {
            if e.kind() != io::ErrorKind::UnexpectedEof {
                log::warn!("client read ended: {e}");
            }
            break;
        }
        let Ok(req) = IpcHeader::read_from_bytes(&hdr_buf) else {
            break;
        };

        match req.msg_type {
            msg::HELLO => {
                let ack = IpcHeader {
                    msg_type: msg::HELLO_ACK,
                    arg0: 0,
                    arg1: device.region_size() as u64,
                    arg2: (u64::from(width) << 32) | u64::from(height),
                };
                if let Err(e) = send_fd_packet(&stream, &ack, device.region_fd()) {
                    log::error!("failed sending memfd via SCM_RIGHTS: {e}");
                    break;
                }
            }
            msg::MMIO_READ => {
                let val = device.mmio_read(req.arg1);
                let ack = IpcHeader {
                    msg_type: msg::MMIO_READ_ACK,
                    arg0: val,
                    arg1: req.arg1,
                    arg2: 0,
                };
                let guard = writer.lock();
                if let Some(mut w) = guard.as_ref()
                    && w.write_all(ack.as_bytes()).is_err()
                {
                    break;
                }
            }
            msg::MMIO_WRITE => {
                device.mmio_write(req.arg1, req.arg0);
                let ack = IpcHeader {
                    msg_type: msg::ACK,
                    arg0: 0,
                    arg1: req.arg1,
                    arg2: 0,
                };
                let guard = writer.lock();
                if let Some(mut w) = guard.as_ref()
                    && w.write_all(ack.as_bytes()).is_err()
                {
                    break;
                }
            }
            msg::DOORBELL => {
                device.doorbell_write(req.arg1);
            }
            msg::RESET => {
                device.reset();
                let ack = IpcHeader {
                    msg_type: msg::ACK,
                    arg0: 0,
                    arg1: 0,
                    arg2: 0,
                };
                let guard = writer.lock();
                if let Some(mut w) = guard.as_ref()
                    && w.write_all(ack.as_bytes()).is_err()
                {
                    break;
                }
            }
            msg::READ_DISPLAY => {
                let max_bytes = (req.arg1 as usize).min(64 * 1024 * 1024);
                frame_buf.resize(max_bytes.max(width as usize * height as usize * 4), 0);
                let (w, h, len) = match device.read_display(&mut frame_buf) {
                    Ok((w, h, _)) => {
                        let need = (w as usize) * (h as usize) * 4;
                        (w, h, need.min(max_bytes))
                    }
                    Err(_) => (width, height, 0),
                };
                let ack = IpcHeader {
                    msg_type: msg::READ_DISPLAY_ACK,
                    arg0: w,
                    arg1: h as u64,
                    arg2: len as u64,
                };
                let guard = writer.lock();
                if let Some(mut wr) = guard.as_ref() {
                    if wr.write_all(ack.as_bytes()).is_err() {
                        break;
                    }
                    if len > 0 && wr.write_all(&frame_buf[..len]).is_err() {
                        break;
                    }
                }
            }
            other => {
                log::warn!("unknown IPC message type {other}");
                break;
            }
        }
    }

    *writer.lock() = None;
}

fn run_server<P: Platform>(
    socket_path: &Path,
    platform: Arc<P>,
    region_bytes: usize,
    width: u32,
    height: u32,
) -> Result<(), String> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create socket parent dir {}: {e}", parent.display()))?;
    }
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("bind {}: {e}", socket_path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    let active_writer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
    let irq_writer = Arc::clone(&active_writer);
    let device = GraphicsDevice::new(
        platform,
        DeviceConfig {
            region_size: region_bytes,
            display_width: width,
            display_height: height,
            raise_irq: Arc::new(move |bits| {
                let guard = irq_writer.lock();
                if let Some(mut w) = guard.as_ref() {
                    let pkt = IpcHeader {
                        msg_type: msg::IRQ_RAISE,
                        arg0: bits,
                        arg1: 0,
                        arg2: 0,
                    };
                    let _ = w.write_all(pkt.as_bytes());
                }
            }),
        },
    )?;

    println!(
        "inferno-vgpu-server listening on {} (gpu={}, region={} MiB, display={}x{})",
        socket_path.display(),
        device.platform().caps().device_name,
        device.region_size() / (1024 * 1024),
        width,
        height
    );

    while !SHUTDOWN.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                serve_connection(stream, &device, &active_writer, width, height);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                log::error!("accept error: {e}");
                break;
            }
        }
    }

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

fn main() {
    install_signal_handlers();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("inferno-vgpu-server: {e}");
            std::process::exit(2);
        }
    };

    let region_bytes = args.region_mb.saturating_mul(1024 * 1024);
    let res = if args.use_fake {
        #[cfg(any(test, feature = "testkit"))]
        {
            let fake = Arc::new(inferno_vgpu_vulkan::core::testkit::FakePlatform::new());
            run_server(&args.socket_path, fake, region_bytes, args.width, args.height)
        }
        #[cfg(not(any(test, feature = "testkit")))]
        {
            Err("built without testkit feature".to_owned())
        }
    } else {
        match RtPlatform::new(None) {
            Ok(rt) => run_server(
                &args.socket_path,
                Arc::new(rt),
                region_bytes,
                args.width,
                args.height,
            ),
            Err(e) => Err(format!("failed to initialize Vulkan RtPlatform: {e}")),
        }
    };

    if let Err(e) = res {
        eprintln!("inferno-vgpu-server: fatal: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inferno_vgpu_graphics::ops::encode_record;
    use inferno_vgpu_graphics::ring::SharedRing;
    use inferno_vgpu_graphics::wire::*;
    use inferno_vgpu_vulkan::core::testkit::FakePlatform;

    fn recv_fd_packet(stream: &UnixStream) -> (IpcHeader, Option<i32>) {
        let mut buf = [0u8; std::mem::size_of::<IpcHeader>()];
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: buf.len(),
        };
        let mut cmsg_buf =
            [0u8; unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) } as usize];
        let mut received_fd = None;
        unsafe {
            let mut msg_hdr: libc::msghdr = std::mem::zeroed();
            msg_hdr.msg_iov = &mut iov;
            msg_hdr.msg_iovlen = 1;
            msg_hdr.msg_control = cmsg_buf.as_mut_ptr().cast::<libc::c_void>();
            msg_hdr.msg_controllen = cmsg_buf.len() as _;
            let n = libc::recvmsg(stream.as_raw_fd(), &mut msg_hdr, 0);
            assert_eq!(n as usize, buf.len());
            let cmsg = libc::CMSG_FIRSTHDR(&msg_hdr);
            if !cmsg.is_null()
                && (*cmsg).cmsg_level == libc::SOL_SOCKET
                && (*cmsg).cmsg_type == libc::SCM_RIGHTS
            {
                let mut fd = -1i32;
                std::ptr::copy_nonoverlapping(
                    libc::CMSG_DATA(cmsg),
                    (&mut fd as *mut i32).cast::<u8>(),
                    std::mem::size_of::<i32>(),
                );
                received_fd = Some(fd);
            }
        }
        (IpcHeader::read_from_bytes(&buf).unwrap(), received_fd)
    }

    #[test]
    fn socket_ipc_memfd_and_ping_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ivg-srv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("test.sock");
        let _ = std::fs::remove_file(&sock);

        let listener = UnixListener::bind(&sock).unwrap();
        let fake = Arc::new(FakePlatform::new());
        let writer_slot: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
        let irq_w = Arc::clone(&writer_slot);
        let device = Arc::new(
            GraphicsDevice::new(
                fake,
                DeviceConfig {
                    region_size: 2 * 1024 * 1024,
                    display_width: 828,
                    display_height: 1792,
                    raise_irq: Arc::new(move |bits| {
                        if let Some(mut w) = irq_w.lock().as_ref() {
                            let pkt = IpcHeader {
                                msg_type: msg::IRQ_RAISE,
                                arg0: bits,
                                arg1: 0,
                                arg2: 0,
                            };
                            let _ = w.write_all(pkt.as_bytes());
                        }
                    }),
                },
            )
            .unwrap(),
        );

        let srv_dev = Arc::clone(&device);
        let srv_thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, &srv_dev, &writer_slot, 828, 1792);
        });

        let mut client = UnixStream::connect(&sock).unwrap();
        let hello = IpcHeader {
            msg_type: msg::HELLO,
            ..Default::default()
        };
        client.write_all(hello.as_bytes()).unwrap();
        let (ack, fd_opt) = recv_fd_packet(&client);
        assert_eq!(ack.msg_type, msg::HELLO_ACK);
        let fd = fd_opt.expect("expected memfd via SCM_RIGHTS");
        let size = ack.arg1 as usize;
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        assert_ne!(mapped, libc::MAP_FAILED);
        let base = mapped.cast::<u8>();

        let ctrl_sub = unsafe { SharedRing::init(base, size, 0x0, 0x1000, 4096) }.unwrap();
        let ctrl_cmp =
            unsafe { SharedRing::init(base, size, 0x10000, 0x11000, 4096) }.unwrap();

        for (off, val) in [
            (regs::CONTROL_SUB_OFF_LO, 0x0u32),
            (regs::CONTROL_CMP_OFF_LO, 0x10000),
            (regs::STATUS, IVG_STATUS_DRIVER_READY),
        ] {
            let wr = IpcHeader {
                msg_type: msg::MMIO_WRITE,
                arg0: val,
                arg1: off,
                arg2: 0,
            };
            client.write_all(wr.as_bytes()).unwrap();
            let mut resp = [0u8; std::mem::size_of::<IpcHeader>()];
            client.read_exact(&mut resp).unwrap();
        }

        let ping = encode_record(control_op::PING, 1, 777, &IvgPingPayload { cookie: 0xBEEF });
        ctrl_sub.push_bytes(&ping).unwrap();
        let db = IpcHeader {
            msg_type: msg::DOORBELL,
            arg0: 0,
            arg1: 0,
            arg2: 0,
        };
        client.write_all(db.as_bytes()).unwrap();

        let mut buf = [0u8; std::mem::size_of::<IvgCompletionRecord>()];
        let mut got = None;
        for _ in 0..200 {
            if let Ok(true) = ctrl_cmp.try_pop_fixed(&mut buf) {
                got = Some(IvgCompletionRecord::read_from_bytes(&buf).unwrap());
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let cmp = got.expect("ping completion over shared memfd");
        assert_eq!(cmp.seqno, 777);
        assert_eq!(cmp.result_u64, 0xBEEF);

        drop(client);
        srv_thread.join().unwrap();
        unsafe {
            libc::munmap(mapped, size);
            libc::close(fd);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
