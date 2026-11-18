//! Typed environment plugin configuration. Runtime registration and descriptor
//! metadata use these identities; options are specific to each built-in.
use crate::{DetectionLevel, SecurityPluginMode};
use serde::{Deserialize, Serialize};
mod descriptors;
pub use descriptors::*;

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginId {
    PiiFilter,
    SecretExfiltration,
    Utf8Guard,
    CredentialBroker,
    LogSanitizer,
    DummyPreEicar,
    DummyPostAllow,
}

impl PluginId {
    pub const ALL: [Self; 7] = [
        Self::PiiFilter,
        Self::SecretExfiltration,
        Self::Utf8Guard,
        Self::CredentialBroker,
        Self::LogSanitizer,
        Self::DummyPreEicar,
        Self::DummyPostAllow,
    ];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PiiFilter => "pii_filter",
            Self::SecretExfiltration => "secret_exfiltration",
            Self::Utf8Guard => "utf8_guard",
            Self::CredentialBroker => "credential_broker",
            Self::LogSanitizer => "log_sanitizer",
            Self::DummyPreEicar => "dummy_pre_eicar",
            Self::DummyPostAllow => "dummy_post_allow",
        }
    }
}

pub trait PluginOptions: Default {
    const ID: PluginId;
    const DEFAULT_MODE: SecurityPluginMode;
    const MODES: &'static [SecurityPluginMode];
    fn default_mode() -> SecurityPluginMode {
        Self::DEFAULT_MODE
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "O: PluginOptions + Deserialize<'de>"))]
pub struct PluginConfiguration<O: PluginOptions> {
    #[serde(default = "O::default_mode")]
    #[cfg_attr(feature = "schema", schema(schema_with = plugin_modes_schema::<O>))]
    pub mode: SecurityPluginMode,
    #[serde(default = "informational")]
    pub detection_level: DetectionLevel,
    #[serde(default)]
    pub options: O,
}

/// The external OpenAPI document must use the same choices/default as option
/// validation, rather than advertising every mode of the internal enum.
#[cfg(feature = "schema")]
fn plugin_modes_schema<O: PluginOptions>() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .enum_values(Some(O::MODES.iter().map(|mode| mode.as_str())))
        .default(Some(O::DEFAULT_MODE.as_str().into()))
        .into()
}
fn informational() -> DetectionLevel {
    DetectionLevel::Informational
}
impl<O: PluginOptions> Default for PluginConfiguration<O> {
    fn default() -> Self {
        Self {
            mode: O::DEFAULT_MODE,
            detection_level: informational(),
            options: O::default(),
        }
    }
}
impl<O: PluginOptions> PluginConfiguration<O> {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !O::MODES.contains(&self.mode) {
            return Err("unsupported environment plugin mode");
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PiiFilterOptions {
    pub email: bool,
    pub phone: bool,
    pub card: bool,
    pub ssn: bool,
}
impl Default for PiiFilterOptions {
    fn default() -> Self {
        Self {
            email: true,
            phone: true,
            card: true,
            ssn: true,
        }
    }
}
/// Directions are relative to the sandbox. These options describe the HTTP
/// inspection implemented today; they do not claim universal protocol support.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PiiFilterConfiguration {
    pub inbound: PluginConfiguration<PiiFilterOptions>,
    pub outbound: PluginConfiguration<PiiFilterOptions>,
}
impl PiiFilterConfiguration {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.inbound.validate()?;
        self.outbound.validate()
    }
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Utf8GuardOptions {
    pub inbound: bool,
    pub outbound: bool,
}
impl Default for Utf8GuardOptions {
    fn default() -> Self {
        Self {
            inbound: true,
            outbound: true,
        }
    }
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CredentialBrokerOptions {
    pub capture_enabled: bool,
}
impl Default for CredentialBrokerOptions {
    fn default() -> Self {
        Self { capture_enabled: true }
    }
}

use SecurityPluginMode::{Allow, Ask, Block, Disable, Rewrite};
const CONTENT_MODES: &[SecurityPluginMode] = &[Disable, Allow, Ask, Block, Rewrite];
macro_rules! options {
    ($name:ty,$id:ident,$default:ident,$modes:expr) => {
        impl PluginOptions for $name {
            const ID: PluginId = PluginId::$id;
            const DEFAULT_MODE: SecurityPluginMode = $default;
            const MODES: &'static [SecurityPluginMode] = $modes;
        }
    };
}
options!(PiiFilterOptions, PiiFilter, Disable, CONTENT_MODES);
options!(Utf8GuardOptions, Utf8Guard, Disable, &[Disable, Allow, Ask, Block]);
options!(CredentialBrokerOptions, CredentialBroker, Rewrite, &[Rewrite]);

macro_rules! empty_options {
    ($name:ident,$id:ident,$default:ident,$modes:expr) => {
        #[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
        #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {}
        options!($name, $id, $default, $modes);
    };
}
empty_options!(SecretExfiltrationOptions, SecretExfiltration, Disable, CONTENT_MODES);
empty_options!(LogSanitizerOptions, LogSanitizer, Rewrite, &[Rewrite]);
empty_options!(DummyPreEicarOptions, DummyPreEicar, Disable, &[Disable, Block]);
empty_options!(DummyPostAllowOptions, DummyPostAllow, Disable, &[Disable, Allow]);

/// Absent entries inherit built-in/corporate resolution; an explicit entry
/// replaces the whole plugin configuration, preserving existing precedence.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentPlugins {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii_filter: Option<PiiFilterConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_exfiltration: Option<PluginConfiguration<SecretExfiltrationOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utf8_guard: Option<PluginConfiguration<Utf8GuardOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_broker: Option<PluginConfiguration<CredentialBrokerOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_sanitizer: Option<PluginConfiguration<LogSanitizerOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dummy_pre_eicar: Option<PluginConfiguration<DummyPreEicarOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dummy_post_allow: Option<PluginConfiguration<DummyPostAllowOptions>>,
}
impl EnvironmentPlugins {
    /// Corporate files merge per plugin, with the first declared entry winning
    /// as a whole. Omitted options/directions never inherit from another file.
    pub fn merge_first_wins(&mut self, fallback: Self) {
        let first = std::mem::take(self);
        *self = Self {
            pii_filter: first.pii_filter.or(fallback.pii_filter),
            secret_exfiltration: first.secret_exfiltration.or(fallback.secret_exfiltration),
            utf8_guard: first.utf8_guard.or(fallback.utf8_guard),
            credential_broker: first.credential_broker.or(fallback.credential_broker),
            log_sanitizer: first.log_sanitizer.or(fallback.log_sanitizer),
            dummy_pre_eicar: first.dummy_pre_eicar.or(fallback.dummy_pre_eicar),
            dummy_post_allow: first.dummy_post_allow.or(fallback.dummy_post_allow),
        };
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(config) = &self.pii_filter {
            config.validate()?;
        }
        if let Some(config) = &self.secret_exfiltration {
            config.validate()?;
        }
        if let Some(config) = &self.utf8_guard {
            config.validate()?;
        }
        if let Some(config) = &self.credential_broker {
            config.validate()?;
        }
        if let Some(config) = &self.log_sanitizer {
            config.validate()?;
        }
        if let Some(config) = &self.dummy_pre_eicar {
            config.validate()?;
        }
        if let Some(config) = &self.dummy_post_allow {
            config.validate()?;
        }
        Ok(())
    }
}
