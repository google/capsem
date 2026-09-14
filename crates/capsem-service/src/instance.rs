//! One running VM as the service sees it: the owner process, its sockets,
//! the profile it booted from, and what it holds for the VM's lifetime.
use super::*;

pub(crate) struct InstanceInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) profile_id: String,
    pub(crate) profile_revision: String,
    pub(crate) profile_payload_hash: String,
    pub(crate) asset_pins: BootAssetPins,
    pub(crate) pid: u32,
    pub(crate) uds_path: PathBuf,
    pub(crate) session_dir: PathBuf,
    pub(crate) ram_mb: u64,
    pub(crate) cpus: u32,
    #[allow(dead_code)]
    pub(crate) start_time: std::time::Instant,
    pub(crate) base_version: String,
    /// Whether this is a persistent (named) VM
    pub(crate) persistent: bool,
    /// Environment variables injected at boot
    #[allow(dead_code)]
    pub(crate) env: Option<std::collections::HashMap<String, String>>,
    /// Sandbox this VM was cloned from, if any
    pub(crate) forked_from: Option<String>,
    /// The VM's address on the private link, held for its whole life.
    pub(crate) private_address: std::net::Ipv4Addr,
    /// What the VM owner shows when it asks for a private connection on the
    /// VM's behalf: minted at spawn, written to the session directory for
    /// the owner alone, matched here. Never reaches the guest.
    pub(crate) owner_secret: String,
}
