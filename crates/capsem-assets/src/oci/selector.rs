//! Image selectors: the one matcher behind both image policies.
//!
//! *Sources* say where bytes may be fetched from and are checked before any
//! registry access, against the reference as requested
//! ([`ImageSelector::matches_source`]). *Admission* says which images may
//! execute and is checked after resolution, against a [`ResolvedImage`],
//! which cannot exist without a digest ([`ImageSelector::matches`]).
//!
//! Selectors compare normalized identities, never strings:
//!
//! - The registry authority folds ASCII case, keeps an explicit port, and
//!   spells Docker Hub (`docker.io`, `index.docker.io`, `registry-1.docker.io`,
//!   or no registry at all) as `docker.io`.
//! - A single-component Docker Hub repository gains `library/`
//!   (`redis` is `docker.io/library/redis`).
//! - Paths are compared by whole components and never fold case. The OCI
//!   grammar makes repository paths lowercase, so an uppercase path is refused.
//! - Empty, `.` and `..` components and a trailing `/` are refused.
//! - A digest must be `sha256:` with 64 lowercase hex digits. When a
//!   reference carries both a tag and a digest, the digest wins and the tag is
//!   dropped; tags never take part in matching.
//!
//! Selector forms:
//!
//! | Form | Matches |
//! | --- | --- |
//! | `registry.company.com[:port]` | every repository on that authority |
//! | `ghcr.io/company/agents/**` | repositories strictly beneath that path, not `agents` itself |
//! | `ghcr.io/company/agents/codex` | that repository, at any digest |
//! | `ghcr.io/company/agents/codex@sha256:…` | that repository at that digest only |
//!
//! A selector without `/` names a registry when it looks like one -- it
//! contains `.` or `:`, or is `localhost` -- the same rule the reference
//! grammar uses for its first component; otherwise it is a Docker Hub
//! repository. `registry/**` is the registry itself. A subtree prefix is a
//! namespace, so `docker.io/library/**` is every official image and never
//! gains a second `library/`. A tag is not a selector: it moves, and
//! admission compares digests.

use std::{fmt, str::FromStr};

use anyhow::{bail, ensure, Context, Result};
use oci_client::Reference;

use super::digest_hex;

const DOCKER_HUB: &str = "docker.io";
const DOCKER_HUB_ALIASES: [&str; 3] = [DOCKER_HUB, "index.docker.io", "registry-1.docker.io"];
const DOCKER_HUB_OFFICIAL: &str = "library";
const SUBTREE: &str = "/**";

/// A normalized registry authority and repository path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repository {
    authority: String,
    path: String,
}

impl Repository {
    /// Normalize a repository whose authority and path are already split.
    /// `official` adds `library/` to a single-component Docker Hub path,
    /// which is right for a repository and wrong for a subtree namespace.
    fn normalized(authority: &str, path: &str, official: bool) -> Result<Self> {
        let authority = authority_identity(authority)?;
        ensure!(
            path.len() <= 255 && path.split('/').all(path_component),
            "invalid repository path {path:?}: components must be lowercase OCI names"
        );
        let path = if official && authority == DOCKER_HUB && !path.contains('/') {
            format!("{DOCKER_HUB_OFFICIAL}/{path}")
        } else {
            path.to_owned()
        };
        Ok(Self { authority, path })
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn contains(&self, descendant: &Self) -> bool {
        self.authority == descendant.authority
            && descendant
                .path
                .strip_prefix(self.path.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

impl fmt::Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.authority, self.path)
    }
}

/// A validated `sha256:` content digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Digest(String);

impl Digest {
    pub fn parse(value: &str) -> Result<Self> {
        digest_hex(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An image as requested, before resolution: the digest is present only when
/// the reference pins one. Any tag has been dropped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageReference {
    repository: Repository,
    digest: Option<Digest>,
}

impl ImageReference {
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn digest(&self) -> Option<&Digest> {
        self.digest.as_ref()
    }

    /// Bind the digest the registry resolved this reference to. A reference
    /// that pinned a digest resolves only to that digest.
    pub fn resolve(self, digest: Digest) -> Result<ResolvedImage> {
        if let Some(pinned) = &self.digest {
            ensure!(
                *pinned == digest,
                "{} pins {pinned} but resolved to {digest}",
                self.repository
            );
        }
        Ok(ResolvedImage {
            repository: self.repository,
            digest,
        })
    }
}

impl FromStr for ImageReference {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let reference: Reference = value.parse().context("invalid OCI reference")?;
        Self::try_from(&reference)
    }
}

/// Normalize a reference the puller already parsed, such as the result of
/// [`super::image_reference`].
impl TryFrom<&Reference> for ImageReference {
    type Error = anyhow::Error;

    fn try_from(reference: &Reference) -> Result<Self> {
        Ok(Self {
            repository: Repository::normalized(reference.registry(), reference.repository(), true)?,
            digest: reference.digest().map(Digest::parse).transpose()?,
        })
    }
}

/// An image after resolution. Admission takes only this type, so an image
/// cannot be admitted without the digest it will run as.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedImage {
    repository: Repository,
    digest: Digest,
}

impl ResolvedImage {
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

impl fmt::Display for ResolvedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.repository, self.digest)
    }
}

/// One scope of an image policy. See the module documentation for the forms.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageSelector {
    /// Every repository on this normalized authority.
    Registry(String),
    /// Repositories strictly beneath this path; the path itself is excluded.
    Subtree(Repository),
    /// Exactly this repository, at any digest.
    Repository(Repository),
    /// Exactly this repository at exactly this digest.
    Image(ResolvedImage),
}

impl ImageSelector {
    /// Admission: may this resolved image execute?
    pub fn matches(&self, image: &ResolvedImage) -> bool {
        self.covers(&image.repository, Some(&image.digest))
    }

    /// Sources: may bytes be fetched for this reference? A reference without
    /// a digest never matches a digest selector, because nothing is known
    /// about what its tag points at until the registry has been asked.
    pub fn matches_source(&self, reference: &ImageReference) -> bool {
        self.covers(&reference.repository, reference.digest.as_ref())
    }

    fn covers(&self, repository: &Repository, digest: Option<&Digest>) -> bool {
        match self {
            Self::Registry(authority) => repository.authority == *authority,
            Self::Subtree(root) => root.contains(repository),
            Self::Repository(exact) => exact == repository,
            Self::Image(image) => image.repository == *repository && digest == Some(&image.digest),
        }
    }
}

impl FromStr for ImageSelector {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let invalid = || format!("invalid image selector {value:?}");
        if let Some(prefix) = value.strip_suffix(SUBTREE) {
            if names_registry(prefix) {
                return Ok(Self::Registry(authority_identity(prefix).with_context(invalid)?));
            }
            ensure!(!prefix.contains('@'), "{}: a subtree cannot pin a digest", invalid());
            ensure!(!has_tag(prefix), "{}: a subtree cannot name a tag", invalid());
            // The reference grammar validates the name; the split is done here
            // because a subtree prefix is a namespace and must not gain the
            // `library/` that grammar adds to a single-component Hub name.
            prefix.parse::<Reference>().with_context(invalid)?;
            let (authority, path) = match prefix.split_once('/') {
                Some((first, rest)) if names_registry(first) => (first, rest),
                _ => (DOCKER_HUB, prefix),
            };
            return Ok(Self::Subtree(
                Repository::normalized(authority, path, false).with_context(invalid)?,
            ));
        }
        if names_registry(value) {
            return Ok(Self::Registry(authority_identity(value).with_context(invalid)?));
        }
        let reference: ImageReference = value.parse().with_context(invalid)?;
        match reference.digest {
            Some(digest) => Ok(Self::Image(ResolvedImage {
                repository: reference.repository,
                digest,
            })),
            None if has_tag(value) => bail!(
                "{}: tags move and admission compares digests; name the repository or pin @sha256:",
                invalid()
            ),
            None => Ok(Self::Repository(reference.repository)),
        }
    }
}

impl fmt::Display for ImageSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(authority) => f.write_str(authority),
            Self::Subtree(root) => write!(f, "{root}{SUBTREE}"),
            Self::Repository(exact) => write!(f, "{exact}"),
            Self::Image(image) => write!(f, "{image}"),
        }
    }
}

/// Admission over a policy. An empty policy admits nothing.
pub fn admits(selectors: &[ImageSelector], image: &ResolvedImage) -> bool {
    selectors.iter().any(|selector| selector.matches(image))
}

/// Sources over a policy. An empty policy allows no source.
pub fn allows_source(selectors: &[ImageSelector], reference: &ImageReference) -> bool {
    selectors.iter().any(|selector| selector.matches_source(reference))
}

/// The reference grammar's test for "this first component is a registry".
fn names_registry(component: &str) -> bool {
    !component.contains('/') && (component.contains(['.', ':']) || component == "localhost")
}

/// Whether the last path component of a name carries `:tag`.
fn has_tag(name: &str) -> bool {
    let name = name.split('@').next().unwrap_or_default();
    name.rsplit('/').next().is_some_and(|last| last.contains(':'))
}

/// Validate and normalize `host[:port]`.
fn authority_identity(value: &str) -> Result<String> {
    let value = value.to_ascii_lowercase();
    let (host, port) = match value.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (value.as_str(), None),
    };
    ensure!(
        host.len() <= 253 && host.split('.').all(host_label),
        "invalid registry host {host:?}"
    );
    if let Some(port) = port {
        ensure!(
            port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u16>().is_ok_and(|port| port != 0),
            "invalid registry port {port:?}"
        );
    }
    Ok(match port {
        None if DOCKER_HUB_ALIASES.contains(&host) => DOCKER_HUB.to_owned(),
        _ => value,
    })
}

fn host_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/// One OCI path component: lowercase alphanumerics joined by `.`, `_`, `-`.
/// This excludes empty, `.` and `..` components by construction.
fn path_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(byte))
}

#[cfg(test)]
mod tests;
