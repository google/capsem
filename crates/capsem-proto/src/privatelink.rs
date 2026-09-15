//! What both ends of a network cable agree on: the ethernet facts of the
//! frames on VSOCK `VSOCK_PORT_NETWORK`, the cable id a pump opens with, and
//! the seat frame the service asks a cable's stream with.
//!
//! A cable is a tap in the guest whose frames cross the pump as `u16`-length
//! records and are switched by the network's confined process. The MAC is a
//! function of the member's address, so the switch derives every port's from
//! its id and checks the source of every frame against it.
use std::net::Ipv4Addr;

/// The ethernet header a frame on the link carries: two MACs, an ethertype.
pub const ETHERNET_HEADER_BYTES: usize = 14;
/// The largest IP packet the link carries: a `u16` frame less its header,
/// which is also the largest MTU Linux gives a tap device.
pub const LINK_MTU: usize = u16::MAX as usize - ETHERNET_HEADER_BYTES;

/// The socket queues of a cable's stream, at both the guest pump and the
/// switch: room for a burst of full frames (32 of them, a few milliseconds
/// at 10 Gb/s), so a hop wakes per batch rather than per frame. Not the
/// published-port size, whose small queues exist to push back on one TCP
/// flow.
pub const CABLE_SOCKET_BUFFER_BYTES: usize = 2 * 1024 * 1024;
// At least a burst of full records, and a bounded amount of kernel memory
// per port: checked where the value is, when it is compiled.
const _: () =
    assert!(CABLE_SOCKET_BUFFER_BYTES >= 16 * (u16::MAX as usize + 2) && CABLE_SOCKET_BUFFER_BYTES <= 8 * 1024 * 1024);

/// The MAC of the member at `address`: locally administered, unicast, and
/// nothing but the address, so it never has to be exchanged.
pub const fn mac_of(address: Ipv4Addr) -> [u8; 6] {
    let [a, b, c, d] = address.octets();
    [0x02, 0xca, a, b, c, d]
}

/// What a guest pump writes first on its VSOCK `VSOCK_PORT_NETWORK`
/// connection: the id of the cable the owner told it to plug. The guest names
/// only its own cables; the owner alone knows which network each one is for.
pub const CABLE_HEADER_BYTES: usize = 4;

pub const fn cable_header(cable: u32) -> [u8; CABLE_HEADER_BYTES] {
    cable.to_be_bytes()
}

/// The cable a pump's connection is for; ids start at one.
pub fn decode_cable_header(bytes: &[u8; CABLE_HEADER_BYTES]) -> Result<u32, String> {
    match u32::from_be_bytes(*bytes) {
        0 => Err("cable id 0 names no cable".into()),
        cable => Ok(cable),
    }
}

/// The guest's tap device for a cable.
pub fn cable_device(cable: u32) -> String {
    format!("cable{cable}")
}

/// A frame on a VM owner's handoff socket: the version, what is asked, and
/// the one-time token the service or a source owner was given for it. The
/// size is the router channel record's, so the frame can carry a descriptor.
pub const SEAT_FRAME_BYTES: usize = 10;
pub const SEAT_FRAME_VERSION: u8 = 1;
/// The service asks a cable's guest stream under a LinkAttach token.
pub const SEAT_LINK: u8 = 5;

pub fn seat_frame(kind: u8, token: u64) -> [u8; SEAT_FRAME_BYTES] {
    let mut frame = [0u8; SEAT_FRAME_BYTES];
    frame[0] = SEAT_FRAME_VERSION;
    frame[1] = kind;
    frame[2..].copy_from_slice(&token.to_be_bytes());
    frame
}

/// The kind and token of a seat frame, or why it is not one.
pub fn decode_seat_frame(bytes: &[u8; SEAT_FRAME_BYTES]) -> Result<(u8, u64), String> {
    if bytes[0] != SEAT_FRAME_VERSION {
        return Err(format!("seat frame version {} is not {SEAT_FRAME_VERSION}", bytes[0]));
    }
    if bytes[1] != SEAT_LINK {
        return Err(format!("seat frame kind {} is not a plug request", bytes[1]));
    }
    Ok((bytes[1], u64::from_be_bytes(bytes[2..].try_into().unwrap())))
}

#[cfg(test)]
mod tests;
