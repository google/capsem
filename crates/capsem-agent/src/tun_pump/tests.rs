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

#[test]
fn options_require_both_ends_and_bound_the_mtu() {
    let parsed = parse_options(["--address", "10.128.0.2", "--peer", "10.128.0.1"].map(String::from)).unwrap();
    assert_eq!(parsed.address, Ipv4Addr::new(10, 128, 0, 2));
    assert_eq!(parsed.peer, Ipv4Addr::new(10, 128, 0, 1));
    assert_eq!(parsed.mtu, MAX_PACKET_BYTES);
    assert_eq!(parsed.prefix, 32, "a bare link routes only the peer");
    assert_eq!(parsed.netmask(), Ipv4Addr::new(255, 255, 255, 255));
    let parsed =
        parse_options(["--address", "10.128.0.2", "--peer", "10.128.0.1", "--mtu", "1500"].map(String::from)).unwrap();
    assert_eq!(parsed.mtu, 1500);
    for bad in [
        vec!["--address", "10.128.0.2"],
        vec!["--peer", "10.128.0.1"],
        vec!["--address", "nope", "--peer", "10.128.0.1"],
        vec!["--address", "10.128.0.2", "--peer", "10.128.0.1", "--mtu", "100"],
        vec!["--address", "10.128.0.2", "--peer", "10.128.0.1", "--mtu", "70000"],
        vec!["--address", "10.128.0.2", "--peer", "10.128.0.1", "--bogus", "1"],
        vec!["--address", "10.128.0.2", "--peer", "10.128.0.1", "--prefix", "0"],
        vec!["--address", "10.128.0.2", "--peer", "10.128.0.1", "--prefix", "33"],
        vec!["--address"],
    ] {
        assert!(parse_options(bad.iter().map(|s| s.to_string())).is_err(), "{bad:?}");
    }
}

#[test]
fn the_pool_prefix_becomes_the_device_netmask() {
    let parsed =
        parse_options(["--address", "10.128.0.2", "--peer", "10.128.0.1", "--prefix", "9"].map(String::from)).unwrap();
    assert_eq!(parsed.prefix, 9);
    assert_eq!(
        parsed.netmask(),
        Ipv4Addr::new(255, 128, 0, 0),
        "10.128.0.0/9 routes into tun0"
    );
}
