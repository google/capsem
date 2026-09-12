//! Keep `capsem-tun` running for the life of the guest.
//!
//! The host names the VM's private address and the pool in the boot
//! environment (`CAPSEM_PRIVATE_ADDRESS`, `CAPSEM_PRIVATE_POOL`); the pump is
//! a separate binary with no authority beyond `tun0` and one VSOCK stream, so
//! it is a child here rather than a thread. When it exits -- the host end
//! closed, the VM resumed, a bug -- it is started again after a bounded
//! pause: a guest without its pump has no private network, and nothing else
//! would notice. A VM the host gave no address gets no pump and no device.
use std::net::Ipv4Addr;
use std::process::Command;
use std::time::Duration;

pub const TUN_BINARY: &str = "/usr/local/bin/capsem-tun";
const FIRST_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);

/// What the host said about this VM's private link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateLink {
    pub address: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub prefix: u8,
}

impl PrivateLink {
    /// From the boot environment; `None` when the host named no address.
    pub fn from_env(env: &[(String, String)]) -> Result<Option<Self>, String> {
        let value = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let Some(address) = value("CAPSEM_PRIVATE_ADDRESS") else {
            return Ok(None);
        };
        let address: Ipv4Addr = address
            .parse()
            .map_err(|_| format!("CAPSEM_PRIVATE_ADDRESS {address:?} is not an IPv4 address"))?;
        let pool = value("CAPSEM_PRIVATE_POOL").ok_or("CAPSEM_PRIVATE_ADDRESS without CAPSEM_PRIVATE_POOL")?;
        let (network, prefix) = pool
            .split_once('/')
            .ok_or_else(|| format!("CAPSEM_PRIVATE_POOL {pool:?} is not network/prefix"))?;
        let network: Ipv4Addr = network
            .parse()
            .map_err(|_| format!("CAPSEM_PRIVATE_POOL {pool:?} has no IPv4 network"))?;
        let prefix: u8 = prefix
            .parse()
            .ok()
            .filter(|prefix| (1..=30).contains(prefix))
            .ok_or_else(|| format!("CAPSEM_PRIVATE_POOL {pool:?} prefix must be 1..=30"))?;
        let mask = u32::MAX << (32 - prefix);
        if address.to_bits() & mask != network.to_bits() & mask {
            return Err(format!(
                "CAPSEM_PRIVATE_ADDRESS {address} is outside CAPSEM_PRIVATE_POOL {pool}"
            ));
        }
        Ok(Some(Self {
            address,
            gateway: Ipv4Addr::from_bits((network.to_bits() & mask) + 1),
            prefix,
        }))
    }

    pub fn arguments(&self) -> Vec<String> {
        vec![
            "--address".into(),
            self.address.to_string(),
            "--peer".into(),
            self.gateway.to_string(),
            "--prefix".into(),
            self.prefix.to_string(),
        ]
    }
}

/// The private link comes up as soon as the host has named the address; the
/// pump outlives every shell and is restarted for as long as this agent
/// runs. The returned line is for the boot log.
pub fn start(boot_env: &[(String, String)]) -> String {
    match PrivateLink::from_env(boot_env) {
        Ok(Some(link)) => {
            let line = format!("private link {}/{} via {}", link.address, link.prefix, link.gateway);
            supervise(link);
            line
        }
        Ok(None) => "no private address: no tun0".into(),
        Err(error) => format!("private link refused: {error}"),
    }
}

/// Start the pump and restart it whenever it exits, for as long as the agent
/// lives. Detached: the agent's boot must not wait on the private link.
pub fn supervise(link: PrivateLink) {
    std::thread::Builder::new()
        .name("capsem-tun-supervisor".into())
        .spawn(move || {
            let mut delay = FIRST_RESTART_DELAY;
            loop {
                let started = std::time::Instant::now();
                match Command::new(TUN_BINARY).args(link.arguments()).status() {
                    Ok(status) => eprintln!("[capsem-agent] capsem-tun exited: {status}"),
                    Err(error) => eprintln!("[capsem-agent] capsem-tun could not start: {error}"),
                }
                // A pump that lived a while earned a fresh backoff; one that
                // dies at once backs off up to the cap so a broken binary
                // does not spin the guest.
                delay = if started.elapsed() > MAX_RESTART_DELAY {
                    FIRST_RESTART_DELAY
                } else {
                    (delay * 2).min(MAX_RESTART_DELAY)
                };
                std::thread::sleep(delay);
            }
        })
        .expect("spawn capsem-tun supervisor");
}

#[cfg(test)]
mod tests;
