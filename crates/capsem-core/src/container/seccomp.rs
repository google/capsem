//! The workload's syscall filter: deny by default, allow what ordinary
//! programs need.
//!
//! The allowlist is the one Docker, containerd and Podman ship: moby's
//! `seccomp/default.json`, vendored unchanged from github.com/moby/profiles
//! (Apache-2.0) at commit `2ceae35d351c156cb5a8efc0fdc4a08cf94569d8`, SHA-256
//! `6416b47770785a41ac59073cdc77d9fe98517df2799dc83ef207e622de3053f6`. It
//! tracks new syscalls (`clone3`, `close_range`, `futex_waitv`, ...) and its
//! argument filters have years of use behind them; a hand-written list fails
//! every syscall it forgets with `EPERM`, in places nobody looks.
//!
//! The file carries conditions -- architecture, capabilities held, minimum
//! kernel -- that Docker's engine resolves before runc sees anything. This
//! module resolves them for the guest's architecture and the capabilities the
//! workload is granted, then applies Capsem's own denials on top. It runs on
//! the host and hands the launcher a finished OCI `linux.seccomp` object.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const MOBY_DEFAULT: &str = include_str!("seccomp/moby_default.json");
/// BLAKE3 of the vendored file, so an edit to it is a visible decision.
pub const MOBY_DEFAULT_BLAKE3: &str = "fb6eae60ed8b80c6584b3dd7078f5a4cba9baca0b76cb225c6454eb2e36866bc";

/// What the workload holds inside its namespaces. `CAP_SYS_ADMIN` is never
/// among them: without it the allowlist already denies `mount`, `setns`,
/// `unshare`, `bpf` and namespace-creating `clone`, and `clone3` returns
/// `ENOSYS` so libc falls back to the filtered `clone`.
pub const WORKLOAD_CAPABILITIES: &[&str] = &[
    "CAP_CHOWN",
    "CAP_DAC_OVERRIDE",
    "CAP_FOWNER",
    "CAP_SETGID",
    "CAP_SETUID",
];

const AF_UNIX: u64 = 1;
const AF_INET: u64 = 2;
const AF_INET6: u64 = 10;
const AF_NETLINK: u64 = 16;
const NETLINK_NETFILTER: u64 = 12;

/// The resolved filter for `arch` (OCI name: `arm64`, `amd64`).
pub fn workload_seccomp(arch: &str) -> Result<Value> {
    let moby: Value = serde_json::from_str(MOBY_DEFAULT).context("parse vendored seccomp allowlist")?;
    let scmp_arch = match arch {
        "arm64" => "SCMP_ARCH_AARCH64",
        "amd64" => "SCMP_ARCH_X86_64",
        other => bail!("no seccomp architecture for {other}"),
    };
    // The native ABI only. The guest kernels are built without 32-bit compat,
    // and a filter that admitted SCMP_ARCH_ARM or SCMP_ARCH_X86 would also
    // admit their multiplexed `socketcall`, around the family rules below.
    anyhow::ensure!(
        moby["archMap"]
            .as_array()
            .context("archMap")?
            .iter()
            .any(|entry| entry["architecture"] == scmp_arch),
        "vendored allowlist has no entry for this architecture"
    );
    let architectures = vec![Value::from(scmp_arch)];

    let mut syscalls = Vec::new();
    for rule in moby["syscalls"].as_array().context("syscalls")? {
        if !applies(rule, arch)? {
            continue;
        }
        let mut names: Vec<Value> = rule["names"].as_array().context("rule names")?.clone();
        // `socket` is re-added below with the families a workload may open;
        // `socketcall` would open any family, unfiltered.
        names.retain(|name| name != "socket" && name != "socketcall");
        if names.is_empty() {
            continue;
        }
        let mut resolved = json!({ "names": names, "action": rule["action"] });
        for key in ["args", "errnoRet"] {
            if let Some(value) = rule.get(key) {
                resolved[key] = value.clone();
            }
        }
        syscalls.push(resolved);
    }
    syscalls.extend(socket_rules());

    Ok(json!({
        "defaultAction": moby["defaultAction"],
        "defaultErrnoRet": moby["defaultErrnoRet"],
        "architectures": architectures,
        "syscalls": syscalls,
    }))
}

/// Whether a moby rule holds for this workload: every `includes` condition
/// met and no `excludes` condition met.
fn applies(rule: &Value, arch: &str) -> Result<bool> {
    let holds = |conditions: &Value| -> Result<bool> {
        let arches = conditions["arches"].as_array();
        let caps = conditions["caps"].as_array();
        let arch_ok = arches.is_none_or(|arches| arches.iter().any(|value| value == arch));
        let caps_ok = caps.is_none_or(|caps| {
            caps.iter()
                .all(|cap| cap.as_str().is_some_and(|cap| WORKLOAD_CAPABILITIES.contains(&cap)))
        });
        if let Some(min) = conditions["minKernel"].as_str() {
            // The guest kernel is 6.18; a rule needing a newer one would be a
            // silent no-op, so it fails loudly instead.
            let (major, minor) = min.split_once('.').context("minKernel")?;
            if (major.parse::<u32>()?, minor.parse::<u32>()?) > (6, 18) {
                bail!("vendored allowlist needs kernel {min}, newer than the guest's");
            }
        }
        Ok(arch_ok && caps_ok)
    };
    let included = match rule.get("includes") {
        Some(conditions) if conditions.as_object().is_some_and(|object| !object.is_empty()) => holds(conditions)?,
        _ => true,
    };
    let excluded = match rule.get("excludes") {
        Some(conditions) if conditions.as_object().is_some_and(|object| !object.is_empty()) => {
            let arches = conditions["arches"].as_array();
            let caps = conditions["caps"].as_array();
            arches.is_some_and(|arches| arches.iter().any(|value| value == arch))
                || caps.is_some_and(|caps| {
                    caps.iter()
                        .any(|cap| cap.as_str().is_some_and(|cap| WORKLOAD_CAPABILITIES.contains(&cap)))
                })
        }
        _ => false,
    };
    Ok(included && !excluded)
}

/// One rule per family, never one rule with several conditions on the same
/// argument: runc turns those into an OR, and "not vsock and not netlink"
/// would allow everything. vsock is absent, so it is denied; netlink is
/// allowed except the netfilter protocol, which reaches nf_tables.
fn socket_rules() -> Vec<Value> {
    let family = |family: u64| {
        json!({
            "names": ["socket"],
            "action": "SCMP_ACT_ALLOW",
            "args": [{"index": 0, "value": family, "op": "SCMP_CMP_EQ"}],
        })
    };
    vec![
        family(AF_UNIX),
        family(AF_INET),
        family(AF_INET6),
        json!({
            "names": ["socket"],
            "action": "SCMP_ACT_ALLOW",
            "args": [
                {"index": 0, "value": AF_NETLINK, "op": "SCMP_CMP_EQ"},
                {"index": 2, "value": NETLINK_NETFILTER, "op": "SCMP_CMP_NE"},
            ],
        }),
    ]
}

#[cfg(test)]
mod tests;
