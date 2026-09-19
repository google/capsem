//! Container launch inputs shared by command clients. VM execution stays in its existing owners.

use anyhow::Result;

pub mod publish;
pub mod stage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortMapping {
    pub host: u16,
    pub guest: u16,
}

impl std::str::FromStr for PortMapping {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        let (host, guest) = value
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("publish expects HOST_PORT:GUEST_PORT"))?;
        let mapping = Self {
            host: host.parse()?,
            guest: guest.parse()?,
        };
        anyhow::ensure!(mapping.guest != 0, "guest port must be nonzero");
        Ok(mapping)
    }
}

pub const LAUNCHER: &[u8] = include_bytes!("../../../guest/artifacts/container/launch.py");
pub const STAGE: &str = ".capsem-image";
/// Where a container sees the VM workspace (the VM's /root share). The
/// launcher mounts it there with the stage hidden, and the files API maps
/// absolute container paths under it back to the workspace.
pub const CONTAINER_WORKSPACE: &str = "/workspace";
pub const LAUNCH_COMMAND: &str = "chmod 555 /root/.capsem-image/launch.py && chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec 'mount --make-rprivate /; cd /newroot; mount --move . /; exec chroot . /usr/bin/python3 /root/.capsem-image/launch.py /root/.capsem-image'";

/// The launcher in the background, the way a boot of a configured VM starts
/// it: detached from the exec that asked, with its output on the console that
/// `capsem logs` reads.
pub fn detached_launch_command() -> String {
    format!(
        "setsid /bin/sh -c '{}' </dev/null >/dev/console 2>&1 &",
        LAUNCH_COMMAND.replace('\'', r"'\''")
    )
}

#[cfg(test)]
mod tests;
