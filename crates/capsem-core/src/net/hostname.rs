//! One host identity for policy, dial, certificate minting and telemetry.
//!
//! The guest chooses the spelling; the resolver decides where it goes. Every
//! place that judges or records a host must see the spelling the resolver
//! acts on, or a rule on the canonical name is evaded by any other one.

use std::net::Ipv4Addr;

/// The host identity policy, dial, telemetry and the leaf cache all share:
/// lowercase, without DNS-root trailing dots, and with an IPv4 address in
/// its dotted-quad form whatever form it arrived in. `Example.COM.`,
/// `example.com`, `0x7f000001`, `127.1` and `127.0.0.1` are each one host
/// to the resolver and must be one host here.
pub fn normalize_host(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    match legacy_ipv4(&host) {
        Some(address) => address.to_string(),
        None => host,
    }
}

/// An IPv4 address in any spelling `inet_aton(3)` accepts, which is what
/// `getaddrinfo` tries before it asks DNS: one to four dot-separated parts,
/// each decimal, `0x` hex or leading-zero octal, the last part filling the
/// bytes that remain. Anything else -- too many parts, an empty part, a
/// digit outside the radix, a value past its width -- is not an address to
/// the resolver either, so it is left to be a name.
pub fn legacy_ipv4(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<u64> = host.split('.').map(parse_part).collect::<Option<_>>()?;
    let (last, head) = parts.split_last()?;
    if head.len() > 3 || head.iter().any(|part| *part > 0xff) {
        return None;
    }
    let tail_bits = 8 * (4 - head.len() as u32);
    if tail_bits < 32 && *last >> tail_bits != 0 {
        return None;
    }
    let mut address: u32 = 0;
    for part in head {
        address = (address << 8) | *part as u32;
    }
    address = address.checked_shl(tail_bits).unwrap_or(0) | u32::try_from(*last).ok()?;
    Some(Ipv4Addr::from(address))
}

fn parse_part(part: &str) -> Option<u64> {
    let (digits, radix) = match part.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None if part.len() > 1 && part.starts_with('0') => (&part[1..], 8),
        None => (part, 10),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| (byte as char).is_digit(radix)) {
        return None;
    }
    u64::from_str_radix(digits, radix)
        .ok()
        .filter(|value| u32::try_from(*value).is_ok())
}

#[cfg(test)]
mod tests;
