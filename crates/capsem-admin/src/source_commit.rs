use std::{fmt, str::FromStr};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SourceCommit(String);

impl SourceCommit {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The runtime revision a release of this commit publishes.
    ///
    /// The workspace version alone would collide across nightly re-releases at
    /// an unchanged version, so every released commit gets its own immutable
    /// identity: `<workspace version>-<first 12 hex of the commit>`.
    /// capsem-admin is built from the released checkout, so its own package
    /// version is the workspace version being released.
    pub(crate) fn runtime_revision(&self) -> String {
        format!("{}-{}", env!("CARGO_PKG_VERSION"), &self.0[..12])
    }
}

impl FromStr for SourceCommit {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Ok(Self(value.to_owned()));
        }
        Err(anyhow!("source commit must be 40-character lowercase hexadecimal"))
    }
}

impl fmt::Display for SourceCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub(crate) fn deserialize_optional<'de, D>(deserializer: D) -> std::result::Result<Option<SourceCommit>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    SourceCommit::from_str(&value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}
