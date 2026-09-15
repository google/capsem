use super::*;
use std::sync::mpsc;

/// A tun device in memory: one packet per read and per write, boundaries
/// preserved, and a definite end when the kernel side is dropped. (A Unix
/// datagram pair would keep the boundaries but never delivers an end.)
struct FakeTun {
    from_kernel: mpsc::Receiver<Vec<u8>>,
    to_kernel: mpsc::Sender<Vec<u8>>,
}

struct Kernel {
    send: mpsc::Sender<Vec<u8>>,
    receive: mpsc::Receiver<Vec<u8>>,
}

fn fake_tun() -> (FakeTun, Kernel) {
    let (send, from_kernel) = mpsc::channel();
    let (to_kernel, receive) = mpsc::channel();
    (FakeTun { from_kernel, to_kernel }, Kernel { send, receive })
}

impl PacketDevice for FakeTun {
    fn read_packet(&mut self, packet: &mut [u8]) -> io::Result<usize> {
        match self.from_kernel.recv() {
            Ok(bytes) if bytes.len() <= packet.len() => {
                packet[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Ok(bytes) => Err(io::Error::other(format!(
                "packet of {} bytes exceeds the buffer",
                bytes.len()
            ))),
            Err(_) => Ok(0),
        }
    }

    fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        self.to_kernel
            .send(packet.to_vec())
            .map_err(|_| io::Error::other("device closed"))
    }
}

#[test]
fn device_packets_become_frames_with_their_length() {
    let (mut device, kernel) = fake_tun();
    let mut wire = Vec::new();
    kernel.send.send(vec![1, 2, 3]).unwrap();
    kernel.send.send(vec![7; 1500]).unwrap();
    drop(kernel);
    device_to_stream(&mut device, &mut wire, 1500).unwrap();
    assert_eq!(&wire[..5], &[0, 3, 1, 2, 3]);
    assert_eq!(&wire[5..7], &[5, 220], "1500 big-endian");
    assert_eq!(wire.len(), 5 + 2 + 1500);
}

#[test]
fn frames_become_device_packets_and_a_clean_end_stops() {
    let (mut device, kernel) = fake_tun();
    let mut cursor = io::Cursor::new(vec![0, 2, 9, 9, 0, 1, 5]);
    stream_to_device(&mut cursor, &mut device, 1500).unwrap();
    assert_eq!(kernel.receive.recv().unwrap(), vec![9, 9]);
    assert_eq!(kernel.receive.recv().unwrap(), vec![5]);
}

/// The host side of a cable as the pump reads it: every read is counted,
/// and each hands over at most `chunk` bytes of what is waiting.
struct HostStream {
    waiting: io::Cursor<Vec<u8>>,
    chunk: usize,
    reads: usize,
}

impl Read for HostStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        let limit = buffer.len().min(self.chunk);
        self.waiting.read(&mut buffer[..limit])
    }
}

fn records(count: usize, size: usize) -> Vec<u8> {
    let mut wire = Vec::new();
    for index in 0..count {
        wire.extend_from_slice(&(size as u16).to_be_bytes());
        wire.extend(std::iter::repeat_n(index as u8, size));
    }
    wire
}

#[test]
fn one_read_from_the_host_delivers_every_whole_frame_it_holds() {
    // Reading each header and each payload on its own cost two syscalls a
    // frame, and the receiving pump sat at a full core at 1.7 Gb/s.
    let (mut device, kernel) = fake_tun();
    let mut host = HostStream {
        waiting: io::Cursor::new(records(100, 1400)),
        chunk: usize::MAX,
        reads: 0,
    };
    stream_to_device(&mut host, &mut device, 1500).unwrap();
    for index in 0..100 {
        assert_eq!(kernel.receive.recv().unwrap(), vec![index as u8; 1400]);
    }
    assert!(host.reads <= 2, "{} reads for 100 frames waiting at once", host.reads);
}

#[test]
fn a_frame_split_across_reads_is_written_whole() {
    let (mut device, kernel) = fake_tun();
    let mut host = HostStream {
        waiting: io::Cursor::new(records(5, 1500)),
        chunk: 7,
        reads: 0,
    };
    stream_to_device(&mut host, &mut device, 1500).unwrap();
    for index in 0..5 {
        assert_eq!(kernel.receive.recv().unwrap(), vec![index as u8; 1500]);
    }
}

#[test]
fn a_frame_beyond_the_mtu_ends_the_link_instead_of_truncating() {
    let (mut device, _kernel) = fake_tun();
    let mut cursor = io::Cursor::new(vec![0x10, 0x00]);
    let error = stream_to_device(&mut cursor, &mut device, 1500).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let mut cursor = io::Cursor::new(vec![0, 0]);
    let error = stream_to_device(&mut cursor, &mut device, 1500).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn a_frame_cut_short_is_an_error_not_a_clean_end() {
    let (mut device, _kernel) = fake_tun();
    let mut cursor = io::Cursor::new(vec![0, 4, 1, 2]);
    let error = stream_to_device(&mut cursor, &mut device, 1500).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn both_directions_round_trip_through_a_stream_pair() {
    let (mut guest_device, guest_kernel) = fake_tun();
    let (mut host_device, host_kernel) = fake_tun();
    let (guest_stream, host_stream) = UnixStream::pair().unwrap();
    let mut guest_writer = guest_stream.try_clone().unwrap();
    let mut host_reader = host_stream;
    let guest_egress = thread::spawn(move || device_to_stream(&mut guest_device, &mut guest_writer, 9000));
    let host_ingress = thread::spawn(move || stream_to_device(&mut host_reader, &mut host_device, 9000));
    for size in [1usize, 576, 9000] {
        guest_kernel.send.send(vec![size as u8; size]).unwrap();
        let packet = host_kernel.receive.recv().unwrap();
        assert_eq!(packet.len(), size);
        assert!(packet.iter().all(|byte| *byte == size as u8));
    }
    drop(guest_kernel);
    guest_egress.join().unwrap().unwrap();
    drop(guest_stream);
    host_ingress.join().unwrap().unwrap();
}

fn options(extra: &[&str]) -> Result<Options, String> {
    parse_options(
        ["--cable", "3", "--address", "10.128.0.2"]
            .iter()
            .chain(extra)
            .map(|s| s.to_string()),
    )
}

#[test]
fn options_require_a_cable_and_an_address_and_bound_the_mtu() {
    let parsed = options(&[]).unwrap();
    assert_eq!(parsed.cable, 3);
    assert_eq!(parsed.device(), "cable3");
    assert_eq!(parsed.address, Ipv4Addr::new(10, 128, 0, 2));
    assert_eq!(parsed.mtu, LINK_MTU, "the largest frame less its ethernet header");
    assert_eq!(parsed.frame_bytes(), MAX_FRAME_BYTES);
    assert_eq!(parsed.mac(), [0x02, 0xca, 10, 128, 0, 2]);
    assert_eq!(parsed.prefix, 32, "a bare cable routes only the peer");
    assert_eq!(parsed.netmask(), Ipv4Addr::new(255, 255, 255, 255));
    assert_eq!(options(&["--mtu", "1500"]).unwrap().mtu, 1500);
    for bad in [
        &["--mtu", "100"][..],
        &["--mtu", "65522"],
        &["--bogus", "1"],
        &["--prefix", "0"],
        &["--prefix", "33"],
    ] {
        assert!(options(bad).is_err(), "{bad:?}");
    }
    for bad in [
        vec!["--address", "10.128.0.2"],
        vec!["--cable", "0", "--address", "10.128.0.2"],
        vec!["--cable", "x", "--address", "10.128.0.2"],
        vec!["--cable", "3"],
        vec!["--cable", "3", "--address", "nope"],
        vec!["--address"],
    ] {
        assert!(parse_options(bad.iter().map(|s| s.to_string())).is_err(), "{bad:?}");
    }
}

#[test]
fn the_network_prefix_becomes_the_device_netmask() {
    let parsed = options(&["--prefix", "24"]).unwrap();
    assert_eq!(parsed.prefix, 24);
    assert_eq!(
        parsed.netmask(),
        Ipv4Addr::new(255, 255, 255, 0),
        "only the network's own subnet routes into its cable"
    );
}

#[test]
fn a_pump_names_its_cable_before_any_frame() {
    let mut wire = Vec::new();
    announce(&mut wire, 3).unwrap();
    assert_eq!(wire, capsem_proto::privatelink::cable_header(3));
}

#[test]
fn the_link_request_declares_ten_gigabit_full_duplex() {
    let request = link_settings_request(LINK_SPEED_MBPS);
    assert_eq!(request.len(), 44, "struct ethtool_cmd");
    assert_eq!(u32::from_ne_bytes(request[..4].try_into().unwrap()), ETHTOOL_SSET);
    let speed = u32::from(u16::from_ne_bytes([request[12], request[13]]))
        | u32::from(u16::from_ne_bytes([request[32], request[33]])) << 16;
    assert_eq!(speed, 10_000);
    assert_eq!(request[14], 1, "full duplex");
    assert_eq!(request[18], 0, "no autonegotiation on a virtual cable");
    let faster = link_settings_request(100_000);
    let speed = u32::from(u16::from_ne_bytes([faster[12], faster[13]]))
        | u32::from(u16::from_ne_bytes([faster[32], faster[33]])) << 16;
    assert_eq!(speed, 100_000, "speeds past 16 bits use speed_hi");
}

#[test]
fn a_tap_frame_is_the_mtu_plus_its_ethernet_header() {
    let (mut device, kernel) = fake_tun();
    let mut wire = Vec::new();
    kernel.send.send(vec![1; 1514]).unwrap();
    drop(kernel);
    device_to_stream(&mut device, &mut wire, 1500 + ETHERNET_HEADER_BYTES).unwrap();
    assert_eq!(wire.len(), 2 + 1514);
    let (mut device, kernel) = fake_tun();
    let mut cursor = io::Cursor::new(wire);
    stream_to_device(&mut cursor, &mut device, 1500 + ETHERNET_HEADER_BYTES).unwrap();
    assert_eq!(kernel.receive.recv().unwrap().len(), 1514);
}
