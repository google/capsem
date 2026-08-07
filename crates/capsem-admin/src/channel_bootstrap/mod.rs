use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

pub(super) fn bootstrap_first_party_channel_source(channel: &str, donor: &Value) -> Result<Value> {
    if !matches!(channel, "stable" | "nightly") {
        bail!("first-party channel bootstrap requires stable or nightly");
    }
    let donor_channel = donor
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bootstrap donor is missing its channel"))?;
    if !matches!(donor_channel, "stable" | "nightly") {
        bail!("bootstrap donor must be a first-party stable or nightly channel");
    }
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
    bootstrapped.insert("channel".to_string(), Value::String(channel.to_string()));
    bootstrapped.insert("status".to_string(), Value::String("current".to_string()));
    bootstrapped.insert("packages".to_string(), Value::Array(packages.clone()));
    bootstrapped.insert("profiles".to_string(), Value::Object(Map::new()));
    Ok(Value::Object(bootstrapped))
}

#[cfg(test)]
mod tests;
