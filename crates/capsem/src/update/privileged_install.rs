//! Verify a package again after sudo, in a root-owned directory.

use super::BinaryInstaller;
use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

// sudo may wait indefinitely for a password. Only the elevated process may
// choose package bytes: copy first, then verify inside its private directory.
const PRIVILEGED_INSTALL_SCRIPT: &str = r#"set -eu
umask 077
source=$1
expected_hash=$2
expected_size=$3
layout=$4
stage=$(/usr/bin/mktemp -d /var/tmp/capsem-install.XXXXXXXXXX)
trap '/bin/rm -rf "$stage"' 0
case "$layout" in
    macos_pkg) package="$stage/installer.pkg" ;;
    linux_deb) package="$stage/installer.deb" ;;
    *) exit 2 ;;
esac
/usr/bin/head -c "$expected_size" "$source" > "$package"
case "$layout" in
    macos_pkg) hash=$(/usr/bin/shasum -a 256 "$package") ;;
    linux_deb) hash=$(/usr/bin/sha256sum "$package") ;;
esac
actual_hash=${hash%% *}
actual_size=$(/usr/bin/wc -c < "$package" | /usr/bin/tr -d '[:space:]')
if [ "$actual_hash" != "$expected_hash" ] || [ "$actual_size" != "$expected_size" ]; then
    echo 'capsem: installer changed before privileged verification' >&2
    exit 1
fi
case "$layout" in
    macos_pkg) /usr/sbin/installer -pkg "$package" -target / ;;
    linux_deb) /usr/bin/apt-get install --yes --allow-downgrades "$package" ;;
esac"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BinaryInstallerApplyPlan {
    commands: Vec<BinaryInstallerApplyCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryInstallerApplyCommand {
    program: String,
    args: Vec<String>,
}

impl BinaryInstallerApplyPlan {
    pub(super) fn command_lines(&self) -> Vec<String> {
        self.commands
            .iter()
            .map(BinaryInstallerApplyCommand::command_line)
            .collect()
    }
}

impl BinaryInstallerApplyCommand {
    fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(super) async fn apply_binary_installer_plan(plan: &BinaryInstallerApplyPlan) -> Result<()> {
    for command in &plan.commands {
        let line = command.command_line();
        info!("applying binary update with package manager: {line}");
        let status = tokio::process::Command::new(&command.program)
            .args(&command.args)
            .status()
            .await
            .with_context(|| format!("run binary update apply command: {line}"))?;
        if !status.success() {
            anyhow::bail!(
                "binary update apply command failed with {status}: {line}. Current installation was left for the package manager to preserve or repair."
            );
        }
    }
    Ok(())
}

pub(super) fn binary_installer_apply_plan(
    installer: &BinaryInstaller,
    path: &Path,
) -> Result<BinaryInstallerApplyPlan> {
    super::validate_binary_installer_metadata(installer)?;
    if !path.is_file() {
        anyhow::bail!("verified installer package is missing: {}", path.display());
    }
    match installer.install_layout.as_str() {
        "macos_pkg" => {
            if !installer.name.ends_with(".pkg") {
                anyhow::bail!("macOS installer must be a .pkg file");
            }
        }
        "linux_deb" => {
            if !installer.name.ends_with(".deb") {
                anyhow::bail!("Linux installer must be a .deb file");
            }
        }
        other => anyhow::bail!("unsupported binary installer layout {other}"),
    }
    Ok(BinaryInstallerApplyPlan {
        commands: vec![BinaryInstallerApplyCommand {
            program: "sudo".to_string(),
            args: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                PRIVILEGED_INSTALL_SCRIPT.to_string(),
                "capsem-update".to_string(),
                path.display().to_string(),
                installer.sha256.to_ascii_lowercase(),
                installer.size.to_string(),
                installer.install_layout.clone(),
            ],
        }],
    })
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b':'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests;
