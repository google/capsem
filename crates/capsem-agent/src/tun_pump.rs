// capsem-tun: the guest end of one network cable.
//
// Opens the cable's tap (`cable<N>`), gives it the address the host leased
// in the network, the MAC that address implies, the network's netmask so
// only that subnet routes into it, a 10 Gb/s full-duplex link and a long
// transmit queue, then pumps ethernet frames between the device and one
// VSOCK connection to the host network endpoint (port 5009). The connection
// opens with the cable id; after that every frame carries a big-endian u16
// length. The kernel does ARP, IP and everything above; the network's switch
// on the host forwards by MAC. Nothing here reads a frame: this is a wire,
// not a stack, and it has no authority beyond the one device and the one
// connection it opens at start. The agent runs one per plugged cable
// (`tun_supervisor` in capsem-agent); the tap exists while this process does.
//
// The frame format is shared with `crates/capsem-network/src/frames.rs`;
// the MTU and the MAC rule with `capsem_proto::privatelink`.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::io::{self, Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use capsem_proto::privatelink::{cable_device, cable_header, mac_of, ETHERNET_HEADER_BYTES, LINK_MTU};
use capsem_proto::VSOCK_PORT_NETWORK;
use nix::libc;
use vsock_io::VSOCK_HOST_CID;

const HEADER_BYTES: usize = 2;

/// What each direction moved, printed when the link ends so a stalled
/// transfer can be read from the log rather than reproduced.
pub struct Counters {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub largest: AtomicU64,
}

impl Counters {
    pub const fn new() -> Self {
        Self {
            frames: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            largest: AtomicU64::new(0),
        }
    }

    fn record(&self, length: usize) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(length as u64, Ordering::Relaxed);
        self.largest.fetch_max(length as u64, Ordering::Relaxed);
    }
}

impl Default for Counters {
    fn default() -> Self {
        Self::new()
    }
}

pub static DEVICE_TO_STREAM: Counters = Counters::new();
pub static STREAM_TO_DEVICE: Counters = Counters::new();
/// A frame's u16 length bounds the frame, and so the device MTU plus its
/// ethernet header.
pub const MAX_FRAME_BYTES: usize = u16::MAX as usize;
/// The link speed a cable declares. A tap reports 10 Mb/s by default, which
/// makes Linux tooling and schedulers treat a private network as slow.
pub const LINK_SPEED_MBPS: u32 = 10_000;
/// Frames the kernel queues for a cable before it drops, sized for bursts
/// at that speed rather than the tap default of 500.
pub const TX_QUEUE_FRAMES: i32 = 10_000;
/// `ETHTOOL_SSET`: the legacy settings call, which the core converts into
/// the tun driver's link settings.
pub const ETHTOOL_SSET: u32 = 0x0000_0002;

/// A `struct ethtool_cmd` setting `speed_mbps`, full duplex and no
/// autonegotiation, in the kernel's native byte order.
pub fn link_settings_request(speed_mbps: u32) -> [u8; 44] {
    let mut request = [0u8; 44];
    request[..4].copy_from_slice(&ETHTOOL_SSET.to_ne_bytes());
    request[12..14].copy_from_slice(&(speed_mbps as u16).to_ne_bytes());
    request[14] = 1; // DUPLEX_FULL
    request[18] = 0; // AUTONEG_DISABLE
    request[32..34].copy_from_slice(&((speed_mbps >> 16) as u16).to_ne_bytes());
    request
}

/// Name the cable on the connection, before any frame.
pub fn announce(stream: &mut impl Write, cable: u32) -> io::Result<()> {
    stream.write_all(&cable_header(cable))
}

/// One frame per read and per write, as a tap device behaves.
pub trait PacketDevice {
    fn read_packet(&mut self, packet: &mut [u8]) -> io::Result<usize>;
    fn write_packet(&mut self, packet: &[u8]) -> io::Result<()>;
}

impl PacketDevice for std::fs::File {
    fn read_packet(&mut self, packet: &mut [u8]) -> io::Result<usize> {
        self.read(packet)
    }

    fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        // A tap write is one frame; a partial write would be a truncated
        // frame, which the kernel refuses rather than splits.
        match self.write(packet)? {
            written if written == packet.len() => Ok(()),
            written => Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("tun accepted {written} of {} bytes", packet.len()),
            )),
        }
    }
}

/// Device frames become records on the stream, until the device ends.
/// `largest` is the biggest frame the device can hand over: its MTU plus
/// the ethernet header.
pub fn device_to_stream(device: &mut impl PacketDevice, stream: &mut impl Write, largest: usize) -> io::Result<()> {
    let mut frame = vec![0u8; HEADER_BYTES + largest];
    loop {
        let length = device.read_packet(&mut frame[HEADER_BYTES..])?;
        if length == 0 {
            return Ok(());
        }
        frame[..HEADER_BYTES].copy_from_slice(&(length as u16).to_be_bytes());
        stream.write_all(&frame[..HEADER_BYTES + length])?;
        DEVICE_TO_STREAM.record(length);
    }
}

/// Records on the stream become device frames, until the stream ends.
/// A frame longer than `largest` is a peer that disagrees about the link
/// and ends the pump, rather than a frame to truncate.
pub fn stream_to_device(stream: &mut impl Read, device: &mut impl PacketDevice, largest: usize) -> io::Result<()> {
    let mut header = [0u8; HEADER_BYTES];
    let mut packet = vec![0u8; largest];
    loop {
        match stream.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
        let length = usize::from(u16::from_be_bytes(header));
        if length == 0 || length > largest {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame of {length} bytes on a link whose frames hold {largest}"),
            ));
        }
        stream.read_exact(&mut packet[..length])?;
        device.write_packet(&packet[..length])?;
        STREAM_TO_DEVICE.record(length);
    }
}

pub struct Options {
    pub cable: u32,
    pub address: Ipv4Addr,
    /// The network's prefix length: with it the kernel routes that subnet,
    /// and only that subnet, into the cable.
    pub prefix: u8,
    pub mtu: usize,
}

impl Options {
    pub fn device(&self) -> String {
        cable_device(self.cable)
    }

    /// The largest frame the device hands over or accepts.
    pub fn frame_bytes(&self) -> usize {
        self.mtu + ETHERNET_HEADER_BYTES
    }

    /// The device's MAC: the one the network's switch derives for this
    /// address, so frames from here are recognisably this member's.
    pub fn mac(&self) -> [u8; 6] {
        mac_of(self.address)
    }

    pub fn netmask(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix)
        })
    }
}

pub fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut cable = None;
    let mut address = None;
    let mut prefix = 32u8;
    let mut mtu = LINK_MTU;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--cable" => {
                cable = Some(
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|cable| *cable > 0)
                        .ok_or_else(|| format!("invalid cable {value}"))?,
                )
            }
            "--address" => address = Some(value.parse().map_err(|_| format!("invalid address {value}"))?),
            "--prefix" => {
                prefix = value.parse().map_err(|_| format!("invalid prefix {value}"))?;
                if !(1..=32).contains(&prefix) {
                    return Err("prefix must be 1..=32".into());
                }
            }
            "--mtu" => {
                mtu = value.parse().map_err(|_| format!("invalid mtu {value}"))?;
                if !(576..=LINK_MTU).contains(&mtu) {
                    return Err(format!("mtu must be 576..={LINK_MTU}"));
                }
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Options {
        cable: cable.ok_or("--cable is required")?,
        address: address.ok_or("--address is required")?,
        prefix,
        mtu,
    })
}

#[cfg(target_os = "linux")]
mod tun {
    //! The device: `/dev/net/tun` plus the SIOC ioctls that name, address
    //! and raise it. Written against the kernel's `ifreq` layout directly;
    //! the musl target has no netlink helper worth a dependency for five
    //! calls made once.
    use super::{link_settings_request, Options, LINK_SPEED_MBPS, TX_QUEUE_FRAMES};
    use nix::libc;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::AsRawFd;

    const TUNSETIFF: libc::Ioctl = 0x4004_54ca;
    const SIOCGIFFLAGS: libc::Ioctl = 0x8913;
    const SIOCSIFFLAGS: libc::Ioctl = 0x8914;
    const SIOCSIFADDR: libc::Ioctl = 0x8916;
    const SIOCSIFNETMASK: libc::Ioctl = 0x891c;
    const SIOCSIFMTU: libc::Ioctl = 0x8922;
    const SIOCSIFHWADDR: libc::Ioctl = 0x8924;
    const SIOCSIFTXQLEN: libc::Ioctl = 0x8943;
    const SIOCETHTOOL: libc::Ioctl = 0x8946;
    const IFF_TAP: u16 = 0x0002;
    /// `ARPHRD_ETHER`: the hardware address family of an ethernet device.
    const ARPHRD_ETHER: u16 = 1;
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

    /// `libc::Ioctl` is the request type on both libcs: `c_ulong` on glibc and
    /// `c_int` on musl, so no request needs a cast that is lossless on only one.
    fn ioctl(fd: &impl AsRawFd, request: libc::Ioctl, argument: &mut [u8; IFREQ_BYTES]) -> io::Result<()> {
        // SAFETY: every request here takes a pointer to an `ifreq` the
        // caller owns for the duration of the call.
        if unsafe { libc::ioctl(fd.as_raw_fd(), request, argument.as_mut_ptr()) } < 0 {
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
        request[NAME_BYTES..NAME_BYTES + 2].copy_from_slice(&(IFF_TAP | IFF_NO_PI).to_ne_bytes());
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
        request[NAME_BYTES..NAME_BYTES + 2].copy_from_slice(&ARPHRD_ETHER.to_ne_bytes());
        request[NAME_BYTES + 2..NAME_BYTES + 8].copy_from_slice(&options.mac());
        ioctl(&socket, SIOCSIFHWADDR, &mut request)?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 8].copy_from_slice(&sockaddr_in(options.netmask()));
        ioctl(&socket, SIOCSIFNETMASK, &mut request)?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 4].copy_from_slice(&(options.mtu as i32).to_ne_bytes());
        ioctl(&socket, SIOCSIFMTU, &mut request)?;
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + 4].copy_from_slice(&TX_QUEUE_FRAMES.to_ne_bytes());
        ioctl(&socket, SIOCSIFTXQLEN, &mut request)?;
        let mut settings = link_settings_request(LINK_SPEED_MBPS);
        let mut request = ifreq(name);
        request[NAME_BYTES..NAME_BYTES + size_of::<usize>()]
            .copy_from_slice(&(settings.as_mut_ptr() as usize).to_ne_bytes());
        ioctl(&socket, SIOCETHTOOL, &mut request)?;
        let mut request = ifreq(name);
        ioctl(&socket, SIOCGIFFLAGS, &mut request)?;
        let flags = u16::from_ne_bytes([request[NAME_BYTES], request[NAME_BYTES + 1]]) | IFF_UP | IFF_RUNNING;
        request[NAME_BYTES..NAME_BYTES + 2].copy_from_slice(&flags.to_ne_bytes());
        ioctl(&socket, SIOCSIFFLAGS, &mut request)
    }

    use std::mem::size_of;
    use std::os::fd::FromRawFd;
}

#[cfg(not(target_os = "linux"))]
mod tun {
    use super::Options;
    use std::fs::File;
    use std::io;

    pub fn open(_: &str) -> io::Result<File> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "tap devices are Linux only"))
    }

    pub fn configure(_: &str, _: &Options) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "tap devices are Linux only"))
    }
}

fn run(options: Options) -> io::Result<()> {
    let name = options.device();
    let device = tun::open(&name)?;
    tun::configure(&name, &options)?;
    let fd = vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_NETWORK)?;
    // A packet stream idles for as long as the guest is quiet; the connect
    // helper's I/O timeouts are for channels with a heartbeat.
    vsock_io::clear_recv_timeout(fd);
    vsock_io::set_socket_timeout(fd, libc::SO_SNDTIMEO, Duration::ZERO);
    // SAFETY: vsock_connect returns a new owned descriptor.
    let mut stream = unsafe { UnixStream::from_raw_fd(fd) };
    announce(&mut stream, options.cable)?;
    capsem_foundation::unix::fd::set_stream_buffers(
        stream.as_fd(),
        capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE,
    )?;
    let mac = options.mac().map(|byte| format!("{byte:02x}")).join(":");
    eprintln!(
        "[capsem-tun] {name} {}/{} {mac} mtu {} {LINK_SPEED_MBPS} Mb/s attached to host port {VSOCK_PORT_NETWORK}",
        options.address, options.prefix, options.mtu
    );
    let mut device_reader = device.try_clone()?;
    let mut stream_writer = stream.try_clone()?;
    let largest = options.frame_bytes();
    // Either direction ending ends the link: a tap read cannot be woken
    // from another thread, so the process exits rather than joins.
    thread::Builder::new().name("capsem-tun-egress".into()).spawn(move || {
        let outcome = device_to_stream(&mut device_reader, &mut stream_writer, largest);
        report("device to host", outcome);
    })?;
    let (mut stream_reader, mut device_writer) = (stream, device);
    report(
        "host to device",
        stream_to_device(&mut stream_reader, &mut device_writer, largest),
    )
}

fn report(direction: &str, outcome: io::Result<()>) -> ! {
    for (name, counters) in [
        ("device to host", &DEVICE_TO_STREAM),
        ("host to device", &STREAM_TO_DEVICE),
    ] {
        eprintln!(
            "[capsem-tun] {name}: {} frames, {} bytes, largest {}",
            counters.frames.load(Ordering::Relaxed),
            counters.bytes.load(Ordering::Relaxed),
            counters.largest.load(Ordering::Relaxed)
        );
    }
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
            eprintln!("usage: capsem-tun --cable N --address A.B.C.D [--prefix N] [--mtu N]");
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
