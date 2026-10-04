//! Container launch inputs shared by command clients. VM execution stays in its existing owners.

use anyhow::Result;

pub mod admission;
pub mod publish;
pub mod seccomp;
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

/// One user-namespace map, for both uids and gids, in OCI's spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct IdMap {
    #[serde(rename = "containerID")]
    pub container_id: u32,
    #[serde(rename = "hostID")]
    pub host_id: u32,
    pub size: u32,
}

/// The workload's user namespace: container ids 0-65535 are VM ids
/// 100000-165535, so container root is no uid the VM trusts. The workspace is
/// idmapped through the same map, so its entries (VM uid 0) are container
/// root's. The launcher refuses a map below VM id 65536.
pub const WORKLOAD_ID_MAP: IdMap = IdMap {
    container_id: 0,
    host_id: 100_000,
    size: 65_536,
};

/// What the workload's cgroup may use: the VM's resources minus what Capsem's
/// own guest services keep, so a workload that exhausts its limit is killed
/// inside its cgroup while the agent, proxies and launcher keep running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WorkloadResources {
    pub memory_bytes: u64,
    /// CPU time per scheduling period, in thousandths of a CPU.
    pub cpu_millis: u64,
    pub pids: u64,
}

/// Memory the runtime keeps for itself: the agent, the network and DNS
/// proxies, the launcher and the page cache they need.
pub const RUNTIME_RESERVE_MB: u64 = 384;
/// CPU the runtime keeps, in thousandths of a CPU.
pub const RUNTIME_RESERVE_CPU_MILLIS: u64 = 250;
/// Below this a workload cannot do useful work; such a VM is refused.
pub const WORKLOAD_MIN_MB: u64 = 256;
/// Enough processes for a compiler or a package manager, few enough that a
/// fork bomb stays inside the cgroup.
pub const WORKLOAD_PIDS: u64 = 4096;

/// The workload's share of a VM with `ram_mb` of memory and `cpus` CPUs.
pub fn workload_resources(ram_mb: u64, cpus: u32) -> Result<WorkloadResources> {
    let memory_mb = ram_mb.saturating_sub(RUNTIME_RESERVE_MB);
    anyhow::ensure!(
        memory_mb >= WORKLOAD_MIN_MB,
        "a VM with {ram_mb} MiB leaves its workload {memory_mb} MiB; it needs at least {} MiB",
        WORKLOAD_MIN_MB + RUNTIME_RESERVE_MB
    );
    anyhow::ensure!(cpus > 0, "a VM needs at least one CPU");
    Ok(WorkloadResources {
        memory_bytes: memory_mb * 1024 * 1024,
        cpu_millis: (u64::from(cpus) * 1000).saturating_sub(RUNTIME_RESERVE_CPU_MILLIS),
        pids: WORKLOAD_PIDS,
    })
}

pub const LAUNCHER: &[u8] = include_bytes!("../../../guest/artifacts/container/launch.py");
pub const STAGE: &str = ".capsem-image";
/// Where a container sees the VM workspace (the VM's /root share). The
/// launcher mounts it there with the stage hidden, and the files API maps
/// absolute container paths under it back to the workspace.
pub const CONTAINER_WORKSPACE: &str = "/workspace";
/// The launcher with `$args`, run where it can work: the agent's exec shell is
/// chrooted, and the kernel refuses to create or join a user namespace from a
/// chrooted task. Escaping to PID 1's root and moving the real root over `/`
/// in a private mount namespace gives the same filesystem without the chroot.
macro_rules! launcher_in_moved_root {
    ($args:literal) => {
        concat!(
            "chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec 'mount --make-rprivate /; cd /newroot; ",
            "mount --move . /; exec chroot . /usr/bin/python3 /root/.capsem-image/launch.py ",
            $args,
            "'"
        )
    };
}

pub const LAUNCH_COMMAND: &str = concat!(
    "chmod 555 /root/.capsem-image/launch.py && ",
    launcher_in_moved_root!("/root/.capsem-image")
);

/// The session terminal's command in an image session: the launcher enters the
/// workload with a login shell, again whenever it ends, and never falls back to
/// the VM. `/etc/capsem-bashrc` hands the PTY to it.
pub const ATTACH_COMMAND: &str = launcher_in_moved_root!("--attach");

/// Set in an image session's boot environment, so its terminal waits for the
/// image it is created with instead of opening a VM shell first. A restarted
/// or forked session finds its staged image (`STAGE/ready`) instead.
pub const WORKLOAD_ENV: &str = "CAPSEM_WORKLOAD";

/// The boot environment of a session, marked as an image session when it has
/// a workload.
pub fn session_env(
    env: Option<std::collections::HashMap<String, String>>,
    workload: bool,
) -> Option<std::collections::HashMap<String, String>> {
    if !workload {
        return env;
    }
    let mut env = env.unwrap_or_default();
    env.insert(WORKLOAD_ENV.to_string(), "1".to_string());
    Some(env)
}

/// `command` run by `/bin/sh -c` inside the running workload, through
/// `runc exec` as the image's own process (user, cwd, env, capabilities,
/// seccomp, cgroup and namespaces). The JSON request travels hex-encoded, so
/// no byte of it is ever shell syntax at either quoting level; the launcher
/// refuses when no workload runs.
pub fn workload_exec_command(command: &str) -> String {
    use std::fmt::Write as _;
    let request = serde_json::json!({ "command": command, "tty": false }).to_string();
    let mut payload = String::with_capacity(request.len() * 2);
    for byte in request.bytes() {
        write!(payload, "{byte:02x}").expect("writing to a String cannot fail");
    }
    format!(launcher_in_moved_root!("--exec {}"), payload)
}

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
