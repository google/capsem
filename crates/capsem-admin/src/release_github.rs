//! The GitHub side of `capsem-admin release`: the `gh` runner that dispatches
//! and watches profile workflows, and the check that a publication identity is
//! still free before any of that starts.

use std::process::Command;
use std::thread;

use anyhow::{anyhow, bail, Context, Result};

use crate::source_commit::SourceCommit;

pub(crate) trait ProfileWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()>;
    fn output(&mut self, args: &[String]) -> Result<String>;
    fn wait_before_poll(&mut self);
}

/// Which commit an existing immutable release was published from.
pub(crate) trait ReleaseLookup {
    /// The source commit of the release tagged `tag`, or `None` if none exists.
    fn release_commit(&mut self, tag: &str) -> Result<Option<String>>;
}

/// Refuse to dispatch a publication whose identity is already taken by a
/// different source commit.
///
/// Releases are immutable, so the hosted lane would build and verify every
/// asset and only then refuse at publication, forty minutes in:
/// `stable/code 0.6.2` had been published from an earlier commit and never
/// activated, and a release of the same revision from main found out at its
/// last step. The same commit is a resume, which publication supports.
pub(crate) fn ensure_publication_identity_is_free(
    lookup: &mut impl ReleaseLookup,
    publication_identity: &str,
    source_commit: &SourceCommit,
) -> Result<()> {
    match lookup.release_commit(publication_identity)? {
        None => Ok(()),
        Some(existing) if existing == source_commit.to_string() => Ok(()),
        Some(existing) => bail!(
            "{publication_identity} is already published from {existing}; releases are immutable, \
             so advance the profile's `revision` in its profile.toml before releasing {source_commit}"
        ),
    }
}

pub(crate) struct GhProfileWorkflowRunner;

impl ProfileWorkflowRunner for GhProfileWorkflowRunner {
    fn run(&mut self, args: &[String]) -> Result<()> {
        let status = Command::new("gh")
            .args(args)
            .status()
            .with_context(|| format!("run gh {}", args.join(" ")))?;
        if !status.success() {
            return Err(anyhow!("gh {} failed with {}", args.join(" "), status));
        }
        Ok(())
    }

    fn output(&mut self, args: &[String]) -> Result<String> {
        let output = Command::new("gh")
            .args(args)
            .output()
            .with_context(|| format!("run gh {}", args.join(" ")))?;
        if !output.status.success() {
            return Err(anyhow!(
                "gh {} failed with {}: {}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).context("gh workflow listing was not UTF-8")
    }

    fn wait_before_poll(&mut self) {
        thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// What `gh release view` prints when the tag has no release.
const RELEASE_NOT_FOUND: &str = "release not found";

impl ReleaseLookup for GhProfileWorkflowRunner {
    fn release_commit(&mut self, tag: &str) -> Result<Option<String>> {
        let output = Command::new("gh")
            .args([
                "release",
                "view",
                tag,
                "--json",
                "targetCommitish",
                "--jq",
                ".targetCommitish",
            ])
            .output()
            .with_context(|| format!("run gh release view {tag}"))?;
        if output.status.success() {
            let commit = String::from_utf8(output.stdout).context("gh release view was not UTF-8")?;
            return Ok(Some(commit.trim().to_string()));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains(RELEASE_NOT_FOUND) {
            return Ok(None);
        }
        // Anything else -- no network, no auth -- is not evidence the identity is free.
        Err(anyhow!(
            "gh release view {tag} failed with {}: {}",
            output.status,
            stderr.trim()
        ))
    }
}

#[cfg(test)]
mod tests;
