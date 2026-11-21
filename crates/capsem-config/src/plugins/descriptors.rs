//! Data-only builtin descriptors. Fields reuse the settings widget contract;
//! nested groups and dependencies do not execute frontend or plugin code.
use super::*;
use crate::{SettingDef, SettingMetadata, SettingType, SettingValue, Widget};
use std::sync::LazyLock;

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStage {
    Preprocess,
    Postprocess,
    Logging,
}
impl PluginStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preprocess => "preprocess",
            Self::Postprocess => "postprocess",
            Self::Logging => "logging",
        }
    }
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDirection {
    Inbound,
    Outbound,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPluginSource {
    Builtin,
    Environment,
    Corporate,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEventFamily {
    Http,
    Model,
    Mcp,
    File,
    Credential,
    Network,
    Dns,
    Process,
    Security,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDetailCapability {
    CredentialInventory,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PluginFieldDependency {
    pub field: String,
    pub not_equals: SettingValue,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PluginField {
    #[serde(flatten)]
    pub setting: SettingDef,
    /// All conditions must hold; this controls presentation, not authority.
    pub dependencies: Vec<PluginFieldDependency>,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginControlGroup {
    /// Configuration path prefix; empty means the plugin's root entry.
    pub path: String,
    pub name: String,
}
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PluginDescriptor {
    pub id: PluginId,
    pub version: u16,
    pub name: &'static str,
    pub description: &'static str,
    pub stage: PluginStage,
    pub event_families: Vec<PluginEventFamily>,
    /// Relative to the sandbox. Coverage remains limited to the listed facts
    /// and protocols; directions never imply universal payload inspection.
    pub directions: Vec<PluginDirection>,
    pub modes: Vec<SecurityPluginMode>,
    pub required: bool,
    pub diagnostic: bool,
    pub groups: Vec<PluginControlGroup>,
    pub fields: Vec<PluginField>,
    pub limitations: Vec<&'static str>,
    pub detail_capabilities: Vec<PluginDetailCapability>,
}

fn path(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        field.into()
    } else {
        format!("{prefix}.{field}")
    }
}
fn dependent(prefix: &str, setting: SettingDef) -> PluginField {
    PluginField {
        setting,
        dependencies: vec![PluginFieldDependency {
            field: path(prefix, "mode"),
            not_equals: SettingValue::Text("disable".into()),
        }],
    }
}
fn controls<O: PluginOptions>(prefix: &str, category: &str) -> Vec<PluginField> {
    vec![
        PluginField {
            setting: SettingDef {
                id: path(prefix, "mode"),
                category: category.into(),
                name: "Mode".into(),
                description: "Disable turns this processing off; other modes use the plugin's supported behavior."
                    .into(),
                setting_type: SettingType::Text,
                default_value: SettingValue::Text(O::DEFAULT_MODE.as_str().into()),
                enabled_by: None,
                metadata: SettingMetadata {
                    choices: O::MODES.iter().map(|mode| mode.as_str().into()).collect(),
                    widget: Some(Widget::Select),
                    builtin: !O::MODES.contains(&SecurityPluginMode::Disable),
                    ..Default::default()
                },
            },
            dependencies: Vec::new(),
        },
        dependent(
            prefix,
            SettingDef {
                id: path(prefix, "detection_level"),
                category: category.into(),
                name: "Detection severity".into(),
                description: "Severity of findings produced when this plugin applies.".into(),
                setting_type: SettingType::Text,
                default_value: SettingValue::Text(PluginConfiguration::<O>::default().detection_level.as_str().into()),
                enabled_by: None,
                metadata: SettingMetadata {
                    choices: ["informational", "low", "medium", "high", "critical"]
                        .map(String::from)
                        .to_vec(),
                    widget: Some(Widget::Select),
                    ..Default::default()
                },
            },
        ),
    ]
}
fn boolean(prefix: &str, category: &str, key: &str, name: &str, description: &str, default: bool) -> PluginField {
    dependent(
        prefix,
        SettingDef {
            id: path(prefix, &format!("options.{key}")),
            category: category.into(),
            name: name.into(),
            description: description.into(),
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(default),
            enabled_by: None,
            metadata: SettingMetadata {
                widget: Some(Widget::Toggle),
                ..Default::default()
            },
        },
    )
}
fn descriptor<O: PluginOptions>(name: &'static str, description: &'static str, stage: PluginStage) -> PluginDescriptor {
    PluginDescriptor {
        id: O::ID,
        version: 1,
        name,
        description,
        stage,
        event_families: Vec::new(),
        directions: vec![PluginDirection::Inbound, PluginDirection::Outbound],
        modes: O::MODES.to_vec(),
        required: !O::MODES.contains(&SecurityPluginMode::Disable),
        diagnostic: false,
        groups: vec![PluginControlGroup {
            path: String::new(),
            name: "Configuration".into(),
        }],
        fields: controls::<O>("", "Configuration"),
        limitations: Vec::new(),
        detail_capabilities: Vec::new(),
    }
}

static DESCRIPTORS: LazyLock<Vec<PluginDescriptor>> = LazyLock::new(|| {
    use PluginEventFamily::{Credential, File, Http, Model};
    use PluginStage::{Logging, Postprocess, Preprocess};
    let mut pii = descriptor::<PiiFilterOptions>(
        "PII Filter",
        "Inspect selected personal data categories independently inbound and outbound.",
        Preprocess,
    );
    pii.event_families = vec![Http];
    pii.limitations = vec![
        "HTTP text only. Does not provide universal model, MCP or file inspection.",
        "Enabled inspection requires bounded HTTP body buffering and can affect streaming.",
    ];
    pii.groups.clear();
    pii.fields.clear();
    let defaults = PiiFilterOptions::default();
    for (prefix, label) in [("inbound", "Inbound to sandbox"), ("outbound", "Outbound from sandbox")] {
        pii.groups.push(PluginControlGroup {
            path: prefix.into(),
            name: label.into(),
        });
        pii.fields.extend(controls::<PiiFilterOptions>(prefix, label));
        for (key, name, description, default) in [
            ("email", "Email addresses", "Detect email address text.", defaults.email),
            (
                "phone",
                "Phone numbers",
                "Detect supported North American phone number text.",
                defaults.phone,
            ),
            (
                "card",
                "Payment cards",
                "Detect supported payment card number text.",
                defaults.card,
            ),
            (
                "ssn",
                "US Social Security numbers",
                "Detect supported US Social Security number text.",
                defaults.ssn,
            ),
        ] {
            pii.fields.push(boolean(prefix, label, key, name, description, default));
        }
    }
    let mut secrets = descriptor::<SecretExfiltrationOptions>(
        "Secret Exfiltration Prevention",
        "Detect supported secret and reference patterns outside recognized provider credential locations.",
        Preprocess,
    );
    secrets.event_families = vec![Http];
    secrets.directions = vec![PluginDirection::Outbound];
    secrets.limitations = vec![
        "Outbound HTTP text only; supported patterns are not a universal secret detector.",
        "Enabled inspection requires bounded HTTP body buffering and can affect streaming.",
    ];
    let mut utf8 = descriptor::<Utf8GuardOptions>(
        "UTF-8 Text Guard",
        "Validate HTTP text encoding while preserving valid Unicode and binary media.",
        Preprocess,
    );
    utf8.event_families = vec![Http];
    utf8.limitations = vec![
        "HTTP text only; binary media is excluded and invalid text is never lossily rewritten.",
        "Enabled inspection requires bounded HTTP text buffering.",
    ];
    let defaults = Utf8GuardOptions::default();
    utf8.fields.extend([
        boolean(
            "",
            "Configuration",
            "inbound",
            "Inbound to sandbox",
            "Inspect inbound HTTP text.",
            defaults.inbound,
        ),
        boolean(
            "",
            "Configuration",
            "outbound",
            "Outbound from sandbox",
            "Inspect outbound HTTP text.",
            defaults.outbound,
        ),
    ]);
    let mut broker = descriptor::<CredentialBrokerOptions>(
        "Credential Broker",
        "Capture supported credentials and request authorized reference substitution.",
        Preprocess,
    );
    broker.event_families = vec![Http, Credential, File];
    broker.fields.push(boolean("", "Configuration", "capture_enabled", "Capture credentials", "Persist supported observed credentials. Reference checks, substitution and sanitization remain active when capture is off.", CredentialBrokerOptions::default().capture_enabled));
    broker.limitations = vec!["Captures recognized provider HTTP credential headers and supplied typed observations; not a universal credential detector.", "References require destination/binding admission and successful upstream substitution."];
    broker.detail_capabilities = vec![PluginDetailCapability::CredentialInventory];
    let mut sanitizer = descriptor::<LogSanitizerOptions>(
        "Log Sanitizer",
        "Sanitize HTTP ledger projections and clear raw credential observations.",
        Logging,
    );
    sanitizer.event_families = vec![Http, Credential, File];
    sanitizer.limitations =
        vec!["Sanitization is a ledger operation; it does not replace upstream credential materialization."];
    let mut pre = descriptor::<DummyPreEicarOptions>(
        "Diagnostic EICAR Block",
        "Diagnostic plugin that detects the EICAR marker in supported event text.",
        Preprocess,
    );
    pre.event_families = vec![File, Http, Model];
    pre.diagnostic = true;
    pre.limitations = vec!["Diagnostic marker matching only; not a malware scanner."];
    let mut post = descriptor::<DummyPostAllowOptions>(
        "Diagnostic Postprocess Allow",
        "Diagnostic postprocess allow request; cannot downgrade an existing block.",
        Postprocess,
    );
    post.event_families = vec![
        Http,
        Model,
        PluginEventFamily::Mcp,
        File,
        Credential,
        PluginEventFamily::Network,
        PluginEventFamily::Dns,
        PluginEventFamily::Process,
        PluginEventFamily::Security,
    ];
    post.diagnostic = true;
    post.limitations =
        vec!["Diagnostic only; disabled by default and never grants credential capture or substitution."];
    vec![pii, secrets, utf8, broker, sanitizer, pre, post]
});

pub fn plugin_descriptors() -> &'static [PluginDescriptor] {
    &DESCRIPTORS
}
pub fn plugin_descriptor(id: PluginId) -> &'static PluginDescriptor {
    DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("every builtin plugin has a descriptor")
}

#[cfg(test)]
mod tests;
