use std::{fmt, str::FromStr};

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FirstPartyChannel {
    Stable,
    Nightly,
}

impl FirstPartyChannel {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "stable" => Ok(Self::Stable),
            "nightly" => Ok(Self::Nightly),
            _ => Err(anyhow!("first-party channel must be stable or nightly")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RetiredGraphSha256(String);

impl RetiredGraphSha256 {
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RetiredGraphSha256 {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("retired graph sha256 must be lowercase 64-hex");
        }
        Ok(Self(value.to_string()))
    }
}

impl fmt::Display for RetiredGraphSha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub(super) fn bootstrap_first_party_channel_source(channel: &str, donor: &Value) -> Result<Value> {
    let channel = FirstPartyChannel::parse(channel)?;
    let donor_channel = donor
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bootstrap donor is missing its channel"))?;
    let donor_channel = FirstPartyChannel::parse(donor_channel)
        .map_err(|_| anyhow!("bootstrap donor must be a first-party stable or nightly channel"))?;
    if donor_channel == channel {
        bail!("bootstrap donor must be a different existing channel");
    }
    if donor.get("status").and_then(Value::as_str) != Some("current") {
        bail!("bootstrap donor must be the current channel source manifest");
    }
    let version = donor
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bootstrap donor is missing its manifest version"))?;
    let packages = donor
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("bootstrap donor packages must be an array"))?;
    if packages.is_empty() {
        bail!("bootstrap donor must contain an official package cohort");
    }
    for package in packages {
        if package.get("status").and_then(Value::as_str) != Some("current") {
            bail!("bootstrap donor packages must all be current");
        }
        let url = package
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("bootstrap donor package is missing its URL"))?;
        if !url.starts_with("https://github.com/google/capsem/releases/download/v") {
            bail!("bootstrap donor package URL is not an official Capsem release");
        }
    }

    let mut bootstrapped = Map::new();
    bootstrapped.insert("version".to_string(), Value::String(version.to_string()));
    bootstrapped.insert("channel".to_string(), Value::String(channel.as_str().to_string()));
    bootstrapped.insert("status".to_string(), Value::String("current".to_string()));
    bootstrapped.insert("packages".to_string(), Value::Array(packages.clone()));
    bootstrapped.insert("profiles".to_string(), Value::Object(Map::new()));
    Ok(Value::Object(bootstrapped))
}

pub(super) fn bootstrap_retired_first_party_channel_source(channel: &str, retired: &Value) -> Result<Value> {
    let channel = FirstPartyChannel::parse(channel)?;
    if retired.get("channel").and_then(Value::as_str) != Some(channel.as_str()) {
        bail!("retired graph must belong to the selected channel");
    }
    if retired.get("status").and_then(Value::as_str) != Some("current") {
        bail!("retired graph must be the current channel source manifest");
    }
    let version = retired
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("retired graph is missing its manifest version"))?;
    let mut bootstrapped = Map::new();
    bootstrapped.insert("version".to_string(), Value::String(version.to_string()));
    bootstrapped.insert("channel".to_string(), Value::String(channel.as_str().to_string()));
    bootstrapped.insert("status".to_string(), Value::String("current".to_string()));
    bootstrapped.insert("packages".to_string(), Value::Array(Vec::new()));
    bootstrapped.insert("profiles".to_string(), Value::Object(Map::new()));
    Ok(Value::Object(bootstrapped))
}

#[cfg(test)]
mod tests;
