/// Generic typed settings system with corp override.
///
/// Each setting has an id, name, description, type, category, default value,
/// and optional `enabled_by` pointer to a parent toggle. Settings are stored
/// in TOML files at:
///   - User: ~/.capsem/user.toml
///   - Corporate: /etc/capsem/corp.toml
///
/// Merge semantics: corp settings override user settings per-key.
/// User can only write user.toml. Corp file is read-only (MDM-distributed).
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Setting ID constants (must match defaults.toml paths)
// ---------------------------------------------------------------------------

pub const SETTING_ANTHROPIC_ALLOW: &str = "ai.anthropic.allow";
pub const SETTING_ANTHROPIC_API_KEY: &str = "ai.anthropic.api_key";
pub const SETTING_OPENAI_ALLOW: &str = "ai.openai.allow";
pub const SETTING_OPENAI_API_KEY: &str = "ai.openai.api_key";
pub const SETTING_GOOGLE_ALLOW: &str = "ai.google.allow";
pub const SETTING_GOOGLE_API_KEY: &str = "ai.google.api_key";
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
    Password,
    Url,
    Email,
    #[serde(rename = "apikey")]
    ApiKey,
    Bool,
    /// File to write to a guest path. Value is `{ path, content }`.
    /// JSON files (.json extension) are validated on save.
    File,
    /// List of strings (e.g. domain patterns, tags).
    StringList,
    /// List of integers.
    IntList,
    /// List of floats.
    FloatList,
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
    RerunWizard,
}

/// MCP server transport protocol.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
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
}

/// Per-rule HTTP method permissions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[derive(Default)]
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
    /// Setting is hidden from the UI but still active for policy building.
    #[serde(default)]
    pub hidden: bool,
    /// Non-removable by user (e.g. built-in MCP servers).
    #[serde(default)]
    pub builtin: bool,
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

/// TOML file format for settings files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub settings: HashMap<String, SettingEntry>,
    /// MCP server configuration (optional section in user.toml / corp.toml).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::policy::McpUserConfig>,
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
}

// ---------------------------------------------------------------------------
// MCP server definitions
// ---------------------------------------------------------------------------

pub fn default_true() -> bool {
    true
}

/// A declarative MCP server definition from defaults.toml, user.toml, or corp.toml.
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
    pub tree: Vec<crate::net::policy_config::tree::SettingsNode>,
    pub issues: Vec<crate::net::policy_config::lint::ConfigIssue>,
    pub presets: Vec<crate::net::policy_config::presets::SecurityPreset>,
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
}
