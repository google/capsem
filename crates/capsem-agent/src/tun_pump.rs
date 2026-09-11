// capsem-tun: the guest end of the private network link.
//
// Opens `tun0`, gives it the address the host assigned, and pumps raw IP
// packets between the device and one VSOCK connection to the host network
// endpoint (port 5009), each packet framed with a big-endian u16 length. The
// kernel routes into tun0; the host's smoltcp terminates. Nothing here reads
// a packet: this is a wire, not a stack, and it has no authority beyond the
// one device and the one connection it opens at start.
//
// The frame format is shared with `crates/capsem-network/src/frames.rs`.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io::{self, Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::process;
use std::thread;
use std::time::Duration;

use capsem_proto::VSOCK_PORT_NETWORK;
use nix::libc;
use vsock_io::VSOCK_HOST_CID;

const HEADER_BYTES: usize = 2;
/// A frame's u16 length bounds the packet, and so the device MTU.
pub const MAX_PACKET_BYTES: usize = u16::MAX as usize;
const DEVICE: &str = "tun0";

/// One packet per read and per write, as a tun device behaves.
pub trait PacketDevice {
    fn read_packet(&mut self, packet: &mut [u8]) -> io::Result<usize>;
    fn write_packet(&mut self, packet: &[u8]) -> io::Result<()>;
}

impl PacketDevice for std::fs::File {
    fn read_packet(&mut self, packet: &mut [u8]) -> io::Result<usize> {
        self.read(packet)
    }

    fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        // A tun write is one packet; a partial write would be a truncated
        // packet, which the kernel refuses rather than splits.
        match self.write(packet)? {
            written if written == packet.len() => Ok(()),
            written => Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("tun accepted {written} of {} bytes", packet.len()),
            )),
        }
    }
}

/// Device packets become frames on the stream, until the device ends.
pub fn device_to_stream(device: &mut impl PacketDevice, stream: &mut impl Write, mtu: usize) -> io::Result<()> {
    let mut frame = vec![0u8; HEADER_BYTES + mtu];
    loop {
        let length = device.read_packet(&mut frame[HEADER_BYTES..])?;
        if length == 0 {
            return Ok(());
        }
        frame[..HEADER_BYTES].copy_from_slice(&(length as u16).to_be_bytes());
        stream.write_all(&frame[..HEADER_BYTES + length])?;
    }
}

/// Frames on the stream become device packets, until the stream ends.
/// A frame longer than the MTU is a peer that disagrees about the link and
/// ends the pump, rather than a packet to truncate.
pub fn stream_to_device(stream: &mut impl Read, device: &mut impl PacketDevice, mtu: usize) -> io::Result<()> {
    let mut header = [0u8; HEADER_BYTES];
    let mut packet = vec![0u8; mtu];
    loop {
        match stream.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let length = usize::from(u16::from_be_bytes(header));
        if length == 0 || length > mtu {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame of {length} bytes on a link with MTU {mtu}"),
            ));
        }
        stream.read_exact(&mut packet[..length])?;
        device.write_packet(&packet[..length])?;
    }
}

pub struct Options {
    pub address: Ipv4Addr,
    pub peer: Ipv4Addr,
    pub mtu: usize,
}

pub fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut address = None;
    let mut peer = None;
    let mut mtu = MAX_PACKET_BYTES;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--address" => address = Some(value.parse().map_err(|_| format!("invalid address {value}"))?),
            "--peer" => peer = Some(value.parse().map_err(|_| format!("invalid peer {value}"))?),
            "--mtu" => {
                mtu = value.parse().map_err(|_| format!("invalid mtu {value}"))?;
                if !(576..=MAX_PACKET_BYTES).contains(&mtu) {
                    return Err(format!("mtu must be 576..={MAX_PACKET_BYTES}"));
                }
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Options {
        address: address.ok_or("--address is required")?,
        peer: peer.ok_or("--peer is required")?,
        mtu,
    })
}

#[cfg(target_os = "linux")]
mod tun {
    //! The device: `/dev/net/tun` plus the SIOC ioctls that name, address
    //! and raise it. Written against the kernel's `ifreq` layout directly;
    //! the musl target has no netlink helper worth a dependency for four
    //! calls made once.
    use super::Options;
    use nix::libc;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::AsRawFd;

    const TUNSETIFF: u32 = 0x4004_54ca;
    const SIOCGIFFLAGS: u32 = 0x8913;
    const SIOCSIFFLAGS: u32 = 0x8914;
    const SIOCSIFADDR: u32 = 0x8916;
    const SIOCSIFDSTADDR: u32 = 0x8918;
    const SIOCSIFMTU: u32 = 0x8922;
    const IFF_TUN: u16 = 0x0001;
    const IFF_NO_PI: u16 = 0x1000;
    const IFF_UP: u16 = 0x0001;
    const IFF_RUNNING: u16 = 0x0040;
    /// `struct ifreq`: 16 bytes of name, then a 24-byte union.
    const IFREQ_BYTES: usize = 40;
    const NAME_BYTES: usize = 16;

    fn ifreq(name: &str) -> [u8; IFREQ_BYTES] {
        let mut request = [0u8; IFREQ_BYTES];
        request[..name.len()].copy_from_slice(name.as_bytes());
        request
    }

    fn ioctl(fd: impl AsRawFd, request: u32, argument: &mut [u8; IFREQ_BYTES]) -> io::Result<()> {
        // SAFETY: every request here takes a pointer to an `ifreq` the
        // caller owns for the duration of the call.
        if unsafe { libc::ioctl(fd.as_raw_fd(), request as _, argument.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn sockaddr_in(address: std::net::Ipv4Addr) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[..2].copy_from_slice(&(libc::AF_INET as u16).to_ne_bytes());
        bytes[4..].copy_from_slice(&address.octets());
        bytes
    }

    pub fn open(name: &str) -> io::Result<File> {
        assert!(name.len() < NAME_BYTES);
        let device = OpenOptions::new().read(true).write(true).open("/dev/net/tun")?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 2].copy_from_slice(&(IFF_TUN | IFF_NO_PI).to_ne_bytes());
        ioctl(&device, TUNSETIFF, &mut request)?;
        Ok(device)
    }

    pub fn configure(name: &str, options: &Options) -> io::Result<()> {
        // SAFETY: a plain datagram socket, used only as an ioctl handle.
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a fresh descriptor this function owns.
        let socket = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 8].copy_from_slice(&sockaddr_in(options.address));
        ioctl(&socket, SIOCSIFADDR, &mut request)?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 8].copy_from_slice(&sockaddr_in(options.peer));
        ioctl(&socket, SIOCSIFDSTADDR, &mut request)?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 4].copy_from_slice(&(options.mtu as i32).to_ne_bytes());
        ioctl(&socket, SIOCSIFMTU, &mut request)?;
        let mut request = ifreq(name);
        ioctl(&socket, SIOCGIFFLAGS, &mut request)?;
        let flags = u16::from_ne_bytes([request[NAME_BYTES], request[NAME_BYTES + 1]]) | IFF_UP | IFF_RUNNING;
        request[NAME_BYTES..NAME_BYTES + 2].copy_from_slice(&flags.to_ne_bytes());
        ioctl(&socket, SIOCSIFFLAGS, &mut request)
    }

    use std::os::fd::FromRawFd;
}

#[cfg(not(target_os = "linux"))]
mod tun {
    use super::Options;
    use std::fs::File;
    use std::io;

    pub fn open(_: &str) -> io::Result<File> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "tun devices are Linux only"))
    }

    pub fn configure(_: &str, _: &Options) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "tun devices are Linux only"))
    }
}

fn run(options: Options) -> io::Result<()> {
    let device = tun::open(DEVICE)?;
    tun::configure(DEVICE, &options)?;
    let fd = vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_NETWORK)?;
    // A packet stream idles for as long as the guest is quiet; the connect
    // helper's I/O timeouts are for channels with a heartbeat.
    vsock_io::clear_recv_timeout(fd);
    vsock_io::set_socket_timeout(fd, libc::SO_SNDTIMEO, Duration::ZERO);
    // SAFETY: vsock_connect returns a new owned descriptor.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    capsem_foundation::unix::fd::set_stream_buffers(
        stream.as_fd(),
        capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
    )?;
    eprintln!(
        "[capsem-tun] {DEVICE} {} -> {} mtu {} attached to host port {VSOCK_PORT_NETWORK}",
        options.address, options.peer, options.mtu
    );
    let mut device_reader = device.try_clone()?;
    let mut stream_writer = stream.try_clone()?;
    let mtu = options.mtu;
    // Either direction ending ends the link: a tun read cannot be woken
    // from another thread, so the process exits rather than joins.
    thread::Builder::new().name("capsem-tun-egress".into()).spawn(move || {
        let outcome = device_to_stream(&mut device_reader, &mut stream_writer, mtu);
        report("device to host", outcome);
    })?;
    let (mut stream_reader, mut device_writer) = (stream, device);
    report(
        "host to device",
        stream_to_device(&mut stream_reader, &mut device_writer, mtu),
    )
}

fn report(direction: &str, outcome: io::Result<()>) -> ! {
    match outcome {
        Ok(()) => {
            eprintln!("[capsem-tun] {direction}: link closed");
            process::exit(0)
        }
        Err(error) => {
            eprintln!("[capsem-tun] {direction}: {error}");
            process::exit(1)
        }
    }
}

fn main() {
    let options = match parse_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("[capsem-tun] {error}");
            eprintln!("usage: capsem-tun --address A.B.C.D --peer A.B.C.D [--mtu N]");
            process::exit(2);
        }
    };
    if let Err(error) = run(options) {
        eprintln!("[capsem-tun] {error}");
        process::exit(1);
    }
}

#[cfg(test)]
#[path = "tun_pump/tests.rs"]
mod tests;
