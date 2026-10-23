//! Typed options shared by the two independent content plugins.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentDirection {
    Inbound,
    Outbound,
    #[default]
    Both,
}

impl ContentDirection {
    pub const fn includes(self, inbound: bool) -> bool {
        matches!(self, Self::Both) || matches!((self, inbound), (Self::Inbound, true) | (Self::Outbound, false))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContentFilterConfig {
    pub direction: ContentDirection,
    pub email: bool,
    pub phone: bool,
    pub card: bool,
    pub ssn: bool,
}

impl Default for ContentFilterConfig {
    fn default() -> Self {
        Self {
            direction: ContentDirection::Both,
            email: true,
            phone: true,
            card: true,
            ssn: true,
        }
    }
}

impl ContentFilterConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
