//! The `capsem update` command line, kept beside the updater it drives.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

// The variant's doc comment in `MiscCommands` is the command's help text, so
// this struct carries none of its own.
#[derive(Args, Debug)]
pub(crate) struct UpdateArgs {
    /// Skip confirmation prompt
    #[arg(long, short)]
    yes: bool,
    /// Check the release channel and refresh update status without applying changes.
    #[arg(long, conflicts_with_all = ["yes", "assets", "manifest", "install_manifest_stdin", "corp"])]
    check: bool,
    /// Refresh only VM assets (kernel/initrd/rootfs) from the release URL.
    /// Useful when an asset-only release ships independently of binaries.
    #[arg(long)]
    assets: bool,
    /// Select a named public release channel (for example stable or nightly).
    #[arg(long, value_name = "NAME", value_parser = validate_channel, conflicts_with = "manifest")]
    channel: Option<String>,
    /// Override the asset manifest endpoint for this update.
    #[arg(long, value_name = "URL", value_parser = validate_manifest_url)]
    manifest: Option<String>,
    /// Read preverified manifest bytes from stdin while --manifest remains
    /// their logical URL. Reserved for the native package handoff.
    #[arg(
        long,
        requires_all = ["manifest", "assets"],
        conflicts_with_all = ["yes", "check", "channel", "corp"],
        hide = true
    )]
    install_manifest_stdin: bool,
    /// Fetch and install corporate policy config from this URL.
    #[arg(long, value_name = "URL", value_parser = validate_corp_url, conflicts_with = "assets")]
    corp: Option<String>,
    /// Parse a staged profile catalog with this binary's profile schema and
    /// exit. An updater runs it on the binary it just installed, so profiles
    /// that ship with that binary are judged by the parser that will read them.
    #[arg(long, value_name = "DIR", exclusive = true, hide = true)]
    validate_profile_catalog: Option<PathBuf>,
}

impl UpdateArgs {
    pub(crate) async fn run(&self) -> Result<()> {
        if let Some(dir) = self.validate_profile_catalog.as_deref() {
            super::validate_profile_catalog_dir(dir)?;
            println!(
                "Profile catalog {} is valid for Capsem {}.",
                dir.display(),
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        super::run_update(
            self.yes,
            self.check,
            self.assets,
            self.channel.as_deref(),
            self.manifest.as_deref(),
            self.install_manifest_stdin,
            self.corp.as_deref(),
        )
        .await
    }
}

fn validate_manifest_url(value: &str) -> std::result::Result<String, String> {
    super::validate_source_url_arg("--manifest", value)
}

fn validate_channel(value: &str) -> std::result::Result<String, String> {
    super::validate_channel_name(value).map_err(|error| error.to_string())
}

fn validate_corp_url(value: &str) -> std::result::Result<String, String> {
    super::validate_source_url_arg("--corp", value)
}

#[cfg(test)]
mod tests;
