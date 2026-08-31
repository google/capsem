/// Generic typed UI settings system with corp constraints.
///
/// Each setting has an id, name, description, type, category, default value,
/// and optional `enabled_by` pointer to a parent toggle. Local UI settings are
/// stored in `settings.toml`. Corporate constraints live in `corp.toml`.
///
/// Merge semantics: corp settings override local settings per-key.
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Setting ID constants (must match defaults.toml paths)
// ---------------------------------------------------------------------------

pub const SETTING_GITHUB_ALLOW: &str = "repository.providers.github.allow";
pub const SETTING_GITHUB_TOKEN: &str = "repository.providers.github.token";
pub const SETTING_GITLAB_ALLOW: &str = "repository.providers.gitlab.allow";
pub const SETTING_GITLAB_TOKEN: &str = "repository.providers.gitlab.token";
pub const SETTING_SSH_PUBLIC_KEY: &str = "vm.environment.ssh.public_key";

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// The data type of a setting (drives UI rendering).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    Text,
    Number,
    Url,
    Email,
    #[serde(rename = "apikey")]
    ApiKey,
    Bool,
    /// File to write to a guest path. Value is `{ path, content }`.
    /// JSON files (.json extension) are validated on save.
    File,
    /// Key-value string map (e.g. env vars, HTTP headers).
    KvMap,
    /// List of strings (e.g. domain patterns, tags).
    StringList,
    /// List of integers.
    IntList,
    /// List of floats.
    FloatList,
    /// An MCP tool discovered from a server.
    McpTool,
}

/// Explicit UI widget override. When set on a setting's metadata,
/// the frontend renders this widget instead of inferring from SettingType.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    Toggle,
    TextInput,
    NumberInput,
    PasswordInput,
    Select,
    FileEditor,
    DomainChips,
    StringChips,
    Slider,
    KvEditor,
}

/// Frontend side effect triggered when a setting value changes.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    ToggleTheme,
}

/// Action identifier for grammar-driven action nodes (buttons/widgets).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    CheckUpdate,
    PresetSelect,
}

/// MCP server transport protocol.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
}

/// Where an MCP tool runs.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpToolOrigin {
    Builtin,
    Remote,
    InVm,
}

/// A setting value (untagged for clean TOML serialization).
///
/// Variant order matters: `#[serde(untagged)]` tries variants top-to-bottom.
/// `File` (a table with `path` + `content`) must come before `Text` (a plain
/// string) so TOML tables like `{ path = "...", content = "..." }` deserialize
/// as `File` rather than failing on `Text`.
/// List variants must come before `Text` so arrays deserialize correctly.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(i64),
    Float(f64),
    File { path: String, content: String },
    KvMap(HashMap<String, String>),
    StringList(Vec<String>),
    IntList(Vec<i64>),
    FloatList(Vec<f64>),
    Text(String),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            SettingValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            SettingValue::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_file(&self) -> Option<(&str, &str)> {
        match self {
            SettingValue::File { path, content } => Some((path, content)),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            SettingValue::Float(f) => Some(*f),
            SettingValue::Number(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_string_list(&self) -> Option<&[String]> {
        match self {
            SettingValue::StringList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_int_list(&self) -> Option<&[i64]> {
        match self {
            SettingValue::IntList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_float_list(&self) -> Option<&[f64]> {
        match self {
            SettingValue::FloatList(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_kv_map(&self) -> Option<&HashMap<String, String>> {
        match self {
            SettingValue::KvMap(m) => Some(m),
            _ => None,
        }
    }
}

/// Per-rule HTTP method permissions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct HttpMethodPermissions {
    /// Optional per-rule domain subset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// Path pattern (e.g., "/repos/*").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub get: bool,
    #[serde(default)]
    pub post: bool,
    #[serde(default)]
    pub put: bool,
    #[serde(default)]
    pub delete: bool,
    /// All methods not listed above.
    #[serde(default)]
    pub other: bool,
}

/// Structured metadata for a setting.
///
/// Note: `skip_serializing_if` is intentionally NOT used on collection fields.
/// The frontend accesses fields like `metadata.choices.length` directly, so
/// omitting empty fields from JSON would cause `undefined.length` TypeErrors.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingMetadata {
    /// Domain patterns for network settings.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Valid values for text choice settings.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Minimum for number settings.
    #[serde(default)]
    pub min: Option<i64>,
    /// Maximum for number settings.
    #[serde(default)]
    pub max: Option<i64>,
    /// HTTP rules (keyed by rule name).
    #[serde(default)]
    pub rules: HashMap<String, HttpMethodPermissions>,
    /// Env var name(s) to inject in the guest when this setting is non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
    /// Whether this setting or section starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
    /// Display format hint (DEPRECATED: use `widget` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Documentation URL (applies to any setting type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Expected token/key prefix hint for the UI (e.g. "ghp_", "sk-ant-").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// File type hint for syntax highlighting (e.g. "json", "bash", "conf").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filetype: Option<String>,
    /// Explicit UI widget override. When set, the frontend renders this widget
    /// instead of inferring from setting_type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<Widget>,
    /// Frontend side effect triggered when the value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect: Option<SideEffect>,
    /// Step increment for number settings (e.g. 1 for integers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
    /// Setting is hidden from the UI but still active for policy building.
    #[serde(default)]
    pub hidden: bool,
    /// Non-removable by user (e.g. built-in MCP servers).
    #[serde(default)]
    pub builtin: bool,
    /// Render as masked input (replaces the old `password` SettingType).
    #[serde(default)]
    pub mask: bool,
    /// Regex pattern for value validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    /// MCP tool origin (builtin, remote, in_vm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<McpToolOrigin>,
}

/// Schema definition for a setting (loaded from defaults.toml at compile time).
pub struct SettingDef {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    /// Parent toggle ID (child is greyed out when parent is off).
    pub enabled_by: Option<String>,
    pub metadata: SettingMetadata,
}

/// A single stored setting entry in TOML.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SettingEntry {
    pub value: SettingValue,
    pub modified: String,
}

/// A registered action that can run after a policy rule matches.
///
/// Matching belongs to CEL/Sigma policy rules. Actions are typed plugin
/// identifiers that receive the matched rule plus the current security event
/// and return the next security event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyActionId {
    CredentialBrokerCapture,
    CredentialBrokerSubstitute,
}

impl PolicyActionId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CredentialBrokerCapture => "credential_broker.capture",
            Self::CredentialBrokerSubstitute => "credential_broker.substitute",
        }
    }

    pub const fn all() -> &'static [Self] {
        &[
            Self::CredentialBrokerCapture,
            Self::CredentialBrokerSubstitute,
        ]
    }
}

impl TryFrom<&str> for PolicyActionId {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "credential_broker.capture" => Ok(Self::CredentialBrokerCapture),
            "credential_broker.substitute" => Ok(Self::CredentialBrokerSubstitute),
            _ => Err(format!("unknown policy action '{value}'")),
        }
    }
}

impl Serialize for PolicyActionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PolicyActionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySubjectValue<'a> {
    String(Cow<'a, str>),
    Bool(bool),
    Present,
}

impl<'a> PolicySubjectValue<'a> {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_ref()),
            Self::Bool(true) => Some("true"),
            Self::Bool(false) => Some("false"),
            Self::Present => None,
        }
    }
}

pub trait PolicySubject {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>>;
}

impl PolicySubject for serde_json::Value {
    fn get_policy_field(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        let mut current = self;
        for segment in field.split('.') {
            current = current.get(segment)?;
        }
        match current {
            serde_json::Value::String(value) => {
                Some(PolicySubjectValue::String(Cow::Borrowed(value.as_str())))
            }
            serde_json::Value::Bool(value) => Some(PolicySubjectValue::Bool(*value)),
            serde_json::Value::Number(value) => {
                Some(PolicySubjectValue::String(Cow::Owned(value.to_string())))
            }
            serde_json::Value::Null
            | serde_json::Value::Array(_)
            | serde_json::Value::Object(_) => Some(PolicySubjectValue::Present),
        }
    }
}

/// TOML file format for settings files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SettingsFile {
    #[serde(default)]
    pub settings: HashMap<String, SettingEntry>,
    /// External rule files shared by user profiles and corporate policy.
    #[serde(default, skip_serializing_if = "RuleFileReferences::is_empty")]
    pub rule_files: RuleFileReferences,
    /// Visible default security rules (`[default.<domain>]`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub default: BTreeMap<String, super::security_rule_profile::SecurityRule>,
    /// Optional corp provisioning refresh policy metadata, e.g. "24h".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_policy: Option<String>,
    /// First-principle profile-owned security rules (`[profiles.rules.*]`).
    #[serde(
        default,
        skip_serializing_if = "super::security_rule_profile::SecurityRuleGroup::is_empty"
    )]
    pub profiles: super::security_rule_profile::SecurityRuleGroup,
    /// First-principle corporate security rules (`[corp.rules.*]`).
    #[serde(
        default,
        skip_serializing_if = "super::security_rule_profile::SecurityRuleGroup::is_empty"
    )]
    pub corp: super::security_rule_profile::SecurityRuleGroup,
    /// Corporate-only integrations around shared rule files.
    #[serde(default, skip_serializing_if = "CorpRuleFileReferences::is_empty")]
    pub corp_rule_files: CorpRuleFileReferences,
    /// Provider-owned rules and endpoint defaults (`[ai.<provider>]`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, super::provider_profile::AiProviderProfile>,
    /// Runtime plugin policy (`[plugins]`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, super::security_rule_profile::SecurityPluginConfig>,
    /// MCP server configuration (optional section in profile/corp TOML).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::McpProfileConfig>,
    /// Corporate-owned network mechanics such as DNS upstreams.
    #[serde(default, skip_serializing_if = "NetworkConfig::is_empty")]
    pub network: NetworkConfig,
}

impl SettingsFile {
    pub fn validate_metadata_contract(&self) -> Result<(), String> {
        for (id, entry) in &self.settings {
            validate_stored_setting_contract(id, &entry.value)?;
        }
        for plugin_id in self.plugins.keys() {
            super::validation::validate_identifier("plugin id", plugin_id)?;
        }
        if let Some(mcp) = &self.mcp {
            mcp.validate("settings")?;
        }
        self.network.validate()?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_bodies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_body_capture: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_upstream_ports: Vec<u16>,
    #[serde(default, skip_serializing_if = "DnsNetworkConfig::is_empty")]
    pub dns: DnsNetworkConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub upstream_overrides: BTreeMap<String, UpstreamOverrideConfig>,
}

impl NetworkConfig {
    pub fn is_empty(&self) -> bool {
        self.log_bodies.is_none()
            && self.max_body_capture.is_none()
            && self.http_upstream_ports.is_empty()
            && self.dns.is_empty()
            && self.upstream_overrides.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.max_body_capture, Some(value) if value > 1024 * 1024) {
            return Err("network.max_body_capture must be at most 1048576".to_string());
        }
        for port in &self.http_upstream_ports {
            if *port == 0 {
                return Err("network.http_upstream_ports must not contain 0".to_string());
            }
        }
        for (target, override_config) in &self.upstream_overrides {
            validate_upstream_override_target(target)?;
            override_config.validate(target)?;
        }
        self.dns.validate()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpstreamOverrideConfig {
    pub dial: String,
    pub protocol: UpstreamOverrideProtocolConfig,
}

impl UpstreamOverrideConfig {
    fn validate(&self, target: &str) -> Result<(), String> {
        self.dial.parse::<std::net::SocketAddr>().map_err(|error| {
            format!("network.upstream_overrides.{target}.dial is invalid: {error}")
        })?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamOverrideProtocolConfig {
    Http,
    Tls,
}

fn validate_upstream_override_target(target: &str) -> Result<(), String> {
    let (host, port) = target.rsplit_once(':').ok_or_else(|| {
        format!("network.upstream_overrides key {target:?} must be exact host:port")
    })?;
    if host.trim().is_empty() {
        return Err(format!(
            "network.upstream_overrides key {target:?} must include a host"
        ));
    }
    let port = port.parse::<u16>().map_err(|error| {
        format!("network.upstream_overrides key {target:?} has invalid port: {error}")
    })?;
    if port == 0 {
        return Err(format!(
            "network.upstream_overrides key {target:?} must not use port 0"
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct DnsNetworkConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstreams: Vec<String>,
}

impl DnsNetworkConfig {
    pub fn is_empty(&self) -> bool {
        self.upstreams.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        for upstream in &self.upstreams {
            upstream.parse::<std::net::SocketAddr>().map_err(|error| {
                format!("network.dns.upstreams entry {upstream:?} is invalid: {error}")
            })?;
        }
        Ok(())
    }
}

pub fn validate_stored_setting_contract(id: &str, value: &SettingValue) -> Result<(), String> {
    if is_brokered_credential_setting_id(id) {
        let Some(value) = value.as_text() else {
            return Err(format!("{id} must be stored as a broker credential ref"));
        };
        if !value.is_empty() && !crate::is_credential_reference(value) {
            return Err(format!(
                "{id} must be empty or stored as a credential:blake3 reference"
            ));
        }
    }
    Ok(())
}

pub fn is_brokered_credential_setting_id(id: &str) -> bool {
    matches!(id, SETTING_GITHUB_TOKEN | SETTING_GITLAB_TOKEN)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct RuleFileReferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<String>,
}

impl RuleFileReferences {
    pub fn is_empty(&self) -> bool {
        self.enforcement.is_none() && self.sigma.is_none()
    }

    pub fn merge_first_wins(&mut self, other: Self) {
        if self.enforcement.is_none() {
            self.enforcement = other.enforcement;
        }
        if self.sigma.is_none() {
            self.sigma = other.sigma;
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct CorpRuleFileReferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<String>,
    /// FIXME: Wire this once corp Sigma export/output delivery is implemented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma_output_endpoint: Option<String>,
    /// FIXME: Wire corporate OpenTelemetry export once remote reporting ships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_telemetry: Option<String>,
    /// FIXME: Wire corporate remote enforcement polling once fleet control ships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_enforcement: Option<String>,
}

impl CorpRuleFileReferences {
    pub fn is_empty(&self) -> bool {
        self.enforcement.is_none()
            && self.sigma.is_none()
            && self.sigma_output_endpoint.is_none()
            && self.open_telemetry.is_none()
            && self.remote_enforcement.is_none()
    }

    pub fn merge_first_wins(&mut self, other: Self) {
        if self.enforcement.is_none() {
            self.enforcement = other.enforcement;
        }
        if self.sigma.is_none() {
            self.sigma = other.sigma;
        }
        if self.sigma_output_endpoint.is_none() {
            self.sigma_output_endpoint = other.sigma_output_endpoint;
        }
        if self.open_telemetry.is_none() {
            self.open_telemetry = other.open_telemetry;
        }
        if self.remote_enforcement.is_none() {
            self.remote_enforcement = other.remote_enforcement;
        }
    }
}

/// Where a setting's effective value came from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicySource {
    #[default]
    Default,
    User,
    Corp,
}

/// A single value change record for audit trail.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub value: serde_json::Value,
    pub source: PolicySource,
}

/// A fully resolved setting (for UI consumption).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ResolvedSetting {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    pub effective_value: SettingValue,
    pub source: PolicySource,
    pub modified: Option<String>,
    pub corp_locked: bool,
    pub enabled_by: Option<String>,
    /// Computed: is the parent toggle on? (true if no parent).
    pub enabled: bool,
    pub metadata: SettingMetadata,
    /// Whether this setting starts collapsed in the UI.
    #[serde(default)]
    pub collapsed: bool,
    /// Value change history (audit trail).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HistoryEntry>,
}

// ---------------------------------------------------------------------------
// MCP server definitions
// ---------------------------------------------------------------------------

pub fn default_true() -> bool {
    true
}

/// A declarative MCP server definition from defaults, profile, or corp TOML.
///
/// MCP servers are auto-injected into AI agent config files (Claude, Gemini, Codex)
/// at boot time. Enterprises can add servers via corp.toml.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct McpServerDef {
    /// TOML key (e.g. "capsem", "internal_tools").
    #[serde(default)]
    pub key: String,
    /// Display name.
    pub name: String,
    /// Help text.
    #[serde(default)]
    pub description: Option<String>,
    /// Transport protocol.
    pub transport: McpTransport,
    /// Command to run (required for stdio transport).
    #[serde(default)]
    pub command: Option<String>,
    /// URL to connect to (required for sse transport).
    #[serde(default)]
    pub url: Option<String>,
    /// Command-line arguments (stdio only).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// HTTP headers (sse only).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Non-removable by user (built-in servers).
    #[serde(default)]
    pub builtin: bool,
    /// Explicit enable/disable.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Where this definition came from.
    #[serde(default)]
    pub source: PolicySource,
    /// Whether corp.toml defines this server (user cannot modify).
    #[serde(default)]
    pub corp_locked: bool,
}

// ---------------------------------------------------------------------------
// Unified settings response
// ---------------------------------------------------------------------------

/// Unified response returned by `load_settings` and `save_settings` commands.
/// Bundles everything the frontend needs in a single IPC call.
#[derive(Serialize, Debug, Clone)]
pub struct SettingsResponse {
    pub tree: Vec<super::tree::SettingsNode>,
    pub issues: Vec<super::lint::ConfigIssue>,
}

// ---------------------------------------------------------------------------
// Guest config and VM settings
// ---------------------------------------------------------------------------

/// A file to write into the guest filesystem at boot.
#[derive(Debug, Clone)]
pub struct GuestFile {
    pub path: String,
    pub content: String,
    pub mode: u32,
}

/// Guest VM configuration (extracted from settings).
#[derive(Debug, Default, Clone)]
pub struct GuestConfig {
    pub env: Option<HashMap<String, String>>,
    pub files: Option<Vec<GuestFile>>,
}

/// VM resource settings (extracted from settings).
#[derive(Debug, Default, Clone)]
pub struct VmSettings {
    pub cpu_count: Option<u32>,
    pub scratch_disk_size_gb: Option<u32>,
    pub ram_gb: Option<u32>,
    pub max_concurrent_vms: Option<u32>,
}

#[cfg(test)]
mod tests;
