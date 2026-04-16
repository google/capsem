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
    RerunWizard,
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

/// TOML file format for settings files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub settings: HashMap<String, SettingEntry>,
    /// MCP server configuration (optional section in user.toml / corp.toml).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::mcp::policy::McpUserConfig>,
    /// User-friendly alias mapping (e.g., "ollama.local" = "http://127.0.0.1:11434")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_aliases: Option<HashMap<String, String>>,
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
    pub max_concurrent_vms: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("k".into(), "v".into());
        m
    }

    #[test]
    fn setting_value_as_bool_returns_value_only_for_bool_variant() {
        assert_eq!(SettingValue::Bool(true).as_bool(), Some(true));
        assert_eq!(SettingValue::Bool(false).as_bool(), Some(false));
        assert_eq!(SettingValue::Number(1).as_bool(), None);
        assert_eq!(SettingValue::Text("x".into()).as_bool(), None);
    }

    #[test]
    fn setting_value_as_number_returns_value_only_for_number_variant() {
        assert_eq!(SettingValue::Number(42).as_number(), Some(42));
        assert_eq!(SettingValue::Float(1.0).as_number(), None);
        assert_eq!(SettingValue::Text("42".into()).as_number(), None);
    }

    #[test]
    fn setting_value_as_text_returns_borrowed_str() {
        assert_eq!(SettingValue::Text("hi".into()).as_text(), Some("hi"));
        assert_eq!(SettingValue::Bool(true).as_text(), None);
    }

    #[test]
    fn setting_value_as_file_returns_tuple() {
        let v = SettingValue::File {
            path: "/tmp/x".into(),
            content: "body".into(),
        };
        assert_eq!(v.as_file(), Some(("/tmp/x", "body")));
        assert_eq!(SettingValue::Bool(true).as_file(), None);
    }

    #[test]
    fn setting_value_as_float_accepts_number_and_float() {
        assert_eq!(SettingValue::Float(1.5).as_float(), Some(1.5));
        // Number -> float coercion.
        assert_eq!(SettingValue::Number(3).as_float(), Some(3.0));
        assert_eq!(SettingValue::Text("1.5".into()).as_float(), None);
    }

    #[test]
    fn setting_value_list_accessors_return_slices() {
        let s = SettingValue::StringList(vec!["a".into(), "b".into()]);
        assert_eq!(s.as_string_list(), Some(&["a".to_string(), "b".to_string()][..]));
        assert_eq!(s.as_int_list(), None);
        assert_eq!(s.as_float_list(), None);

        let i = SettingValue::IntList(vec![1, 2]);
        assert_eq!(i.as_int_list(), Some(&[1i64, 2][..]));
        assert_eq!(i.as_string_list(), None);

        let f = SettingValue::FloatList(vec![1.0, 2.5]);
        assert_eq!(f.as_float_list(), Some(&[1.0f64, 2.5][..]));
        assert_eq!(f.as_int_list(), None);
    }

    #[test]
    fn setting_value_as_kv_map_returns_map() {
        let m = make_map();
        let v = SettingValue::KvMap(m.clone());
        assert_eq!(v.as_kv_map(), Some(&m));
        assert_eq!(SettingValue::Bool(true).as_kv_map(), None);
    }

    #[test]
    fn setting_value_deserializes_file_before_text() {
        // File variant must win over Text when input is a table.
        let toml = r#"path = "/etc/x"
content = "hello""#;
        let v: SettingValue = toml::from_str(toml).unwrap();
        match v {
            SettingValue::File { path, content } => {
                assert_eq!(path, "/etc/x");
                assert_eq!(content, "hello");
            }
            other => panic!("expected File variant, got {other:?}"),
        }
    }

    #[test]
    fn setting_value_deserializes_string_list_before_text() {
        let v: SettingValue = toml::from_str("value = [\"a\", \"b\"]")
            .and_then(|t: toml::Value| toml::Value::try_into(t["value"].clone()))
            .unwrap();
        match v {
            SettingValue::StringList(list) => assert_eq!(list, vec!["a", "b"]),
            other => panic!("expected StringList, got {other:?}"),
        }
    }

    #[test]
    fn default_true_helper_returns_true() {
        assert!(default_true());
    }

    #[test]
    fn policy_source_default_is_default_variant() {
        assert_eq!(PolicySource::default(), PolicySource::Default);
    }

    #[test]
    fn http_method_permissions_default_all_off() {
        let p = HttpMethodPermissions::default();
        assert!(!p.get && !p.post && !p.put && !p.delete && !p.other);
        assert!(p.domains.is_empty());
        assert!(p.path.is_none());
    }

    #[test]
    fn settings_file_default_has_empty_settings_and_no_mcp() {
        let f = SettingsFile::default();
        assert!(f.settings.is_empty());
        assert!(f.mcp.is_none());
    }

    #[test]
    fn setting_value_round_trips_through_json() {
        let cases = vec![
            SettingValue::Bool(true),
            SettingValue::Number(7),
            SettingValue::Float(2.5),
            SettingValue::Text("hello".into()),
            SettingValue::StringList(vec!["a".into()]),
            SettingValue::IntList(vec![1, 2, 3]),
            SettingValue::FloatList(vec![1.0, 2.0]),
            SettingValue::KvMap(make_map()),
            SettingValue::File {
                path: "/x".into(),
                content: "y".into(),
            },
        ];
        for v in cases {
            let j = serde_json::to_string(&v).unwrap();
            let back: SettingValue = serde_json::from_str(&j).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn enum_variants_serialize_with_snake_case() {
        assert_eq!(
            serde_json::to_string(&SettingType::ApiKey).unwrap(),
            "\"apikey\""
        );
        assert_eq!(
            serde_json::to_string(&SettingType::KvMap).unwrap(),
            "\"kv_map\""
        );
        assert_eq!(
            serde_json::to_string(&Widget::PasswordInput).unwrap(),
            "\"password_input\""
        );
        assert_eq!(
            serde_json::to_string(&SideEffect::ToggleTheme).unwrap(),
            "\"toggle_theme\""
        );
        assert_eq!(
            serde_json::to_string(&ActionKind::CheckUpdate).unwrap(),
            "\"check_update\""
        );
        assert_eq!(
            serde_json::to_string(&McpTransport::Stdio).unwrap(),
            "\"stdio\""
        );
        assert_eq!(
            serde_json::to_string(&McpToolOrigin::InVm).unwrap(),
            "\"in_vm\""
        );
        assert_eq!(
            serde_json::to_string(&PolicySource::Corp).unwrap(),
            "\"corp\""
        );
    }
}
