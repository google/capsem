//! Per-viewer transport choices; never VM state or authentication policy.

#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DisplayProfile {
    #[default]
    Auto,
    Local,
    Remote,
}

impl DisplayProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    pub(crate) fn resolve(self, parent: &str) -> Self {
        match self {
            Self::Auto => {
                let local = parent
                    .parse::<http::Uri>()
                    .ok()
                    .and_then(|uri| uri.host().map(crate::cors::is_loopback_host))
                    .unwrap_or(false);
                if local {
                    Self::Local
                } else {
                    Self::Remote
                }
            }
            explicit => explicit,
        }
    }

    pub(crate) fn from_form(body: &str) -> anyhow::Result<Self> {
        let mut profiles = url::form_urlencoded::parse(body.as_bytes())
            .filter_map(|(key, value)| (key == "display_profile").then_some(value));
        let profile = match profiles.next().as_deref().unwrap_or("auto") {
            "auto" => Self::Auto,
            "local" => Self::Local,
            "remote" => Self::Remote,
            _ => anyhow::bail!("invalid display profile"),
        };
        anyhow::ensure!(profiles.next().is_none(), "duplicate display profile");
        Ok(profile)
    }
}

#[cfg(test)]
mod tests;
