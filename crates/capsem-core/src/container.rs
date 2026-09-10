//! Container launch inputs shared by command clients. VM execution stays in its existing owners.

use anyhow::Result;

pub mod publish;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub const LAUNCH_COMMAND: &str = "chmod 555 /root/.capsem-image/launch.py && chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec 'mount --make-rprivate /; cd /newroot; mount --move . /; exec chroot . /usr/bin/python3 /root/.capsem-image/launch.py /root/.capsem-image'";

/// Qualified references opt into images; ordinary existing shell commands stay commands.
pub fn image_name(input: &str) -> Result<Option<String>> {
    let first = input.split('/').next().unwrap_or_default();
    let qualified = input.contains('/')
        && !matches!(first, "" | "." | "..")
        && !first.chars().any(char::is_whitespace)
        && (first.contains('.') || first.contains(':') || first == "localhost");
    if !qualified && !input.contains("://") {
        return Ok(None);
    }
    let reference = capsem_assets::oci::image_reference(input)?;
    let name = reference
        .repository()
        .rsplit('/')
        .next()
        .unwrap_or("container")
        .chars()
        .take(48)
        .map(|c| if c == '.' { '-' } else { c })
        .collect();
    Ok(Some(name))
}

pub fn available_name(base: &str, existing: &[String]) -> Result<String> {
    for index in 1..10_000 {
        let candidate = if index == 1 {
            base.to_owned()
        } else {
            format!("{base}-{index}")
        };
        if !existing.iter().any(|name| name.eq_ignore_ascii_case(&candidate)) {
            return Ok(candidate);
        }
    }
    anyhow::bail!("no available VM name for image {base}")
}

#[cfg(test)]
mod tests;
