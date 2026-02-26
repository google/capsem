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
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::domain_policy::{Action, DomainPolicy};
use super::http_policy::{HttpPolicy, HttpRule};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// The data type of a setting (drives UI rendering).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    Text,
    Number,
    Password,
    Url,
    Email,
    ApiKey,
    Bool,
    /// File content to write to a guest path (declared in metadata.guest_path).
    /// JSON files (.json) are validated on save.
    File,
}

/// A setting value (untagged for clean TOML serialization).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Number(i64),
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
}

/// Per-rule HTTP method permissions.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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

impl Default for HttpMethodPermissions {
    fn default() -> Self {
        Self {
            domains: Vec::new(),
            path: None,
            get: false,
            post: false,
            put: false,
            delete: false,
            other: false,
        }
    }
}

/// Structured metadata for a setting.
///
/// Note: `skip_serializing_if` is intentionally NOT used here. The frontend
/// accesses fields like `metadata.choices.length` directly, so omitting empty
/// fields from JSON would cause `undefined.length` TypeErrors in the UI.
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
    /// Guest file path for File-type settings (e.g. "/root/.gemini/settings.json").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_path: Option<String>,
    /// Env var name(s) to inject in the guest when this setting is non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_vars: Vec<String>,
}

/// Schema definition for a setting (compile-time registry).
pub struct SettingDef {
    pub id: &'static str,
    pub category: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub setting_type: SettingType,
    pub default_value: SettingValue,
    /// Parent toggle ID (child is greyed out when parent is off).
    pub enabled_by: Option<&'static str>,
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
}

/// Where a setting's effective value came from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicySource {
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
    pub scratch_disk_size_gb: Option<u32>,
}

// ---------------------------------------------------------------------------
// Setting registry
// ---------------------------------------------------------------------------

/// Returns the compile-time setting definitions.
pub fn setting_definitions() -> Vec<SettingDef> {
    vec![
        // -- AI Providers --
        SettingDef {
            id: "ai.anthropic.allow",
            category: "AI Providers",
            name: "Allow Anthropic",
            description: "Enable API access to Anthropic (api.anthropic.com).",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(false),
            enabled_by: None,
            metadata: SettingMetadata {
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        post: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.anthropic.api_key",
            category: "AI Providers",
            name: "Anthropic API Key",
            description: "API key for Anthropic. Injected as ANTHROPIC_API_KEY env var.",
            setting_type: SettingType::ApiKey,
            default_value: SettingValue::Text(String::new()),
            enabled_by: Some("ai.anthropic.allow"),
            metadata: SettingMetadata {
                env_vars: vec!["ANTHROPIC_API_KEY".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.anthropic.domains",
            category: "AI Providers",
            name: "Anthropic Domains",
            description: "Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("*.anthropic.com, *.claude.com".into()),
            enabled_by: Some("ai.anthropic.allow"),
            metadata: SettingMetadata::default(),
        },
        SettingDef {
            id: "ai.anthropic.claude.settings_json",
            category: "AI Providers",
            name: "Claude Code settings.json",
            description: "Content for ~/.claude/settings.json. Bypass permissions, disable telemetry/updates for sandboxed execution.",
            setting_type: SettingType::File,
            default_value: SettingValue::Text(r#"{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}"#.into()),
            enabled_by: Some("ai.anthropic.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.claude/settings.json".into()), ..Default::default() },
        },
        SettingDef {
            id: "ai.anthropic.claude.state_json",
            category: "AI Providers",
            name: "Claude Code state (.claude.json)",
            description: "Content for ~/.claude.json. Skips onboarding, trust dialogs, and keybinding prompts.",
            setting_type: SettingType::File,
            default_value: SettingValue::Text(r#"{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true,"hasTrustDialogHooksAccepted":true,"shiftEnterKeyBindingInstalled":true,"theme":"dark"}"#.into()),
            enabled_by: Some("ai.anthropic.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.claude.json".into()), ..Default::default() },
        },
        SettingDef {
            id: "ai.openai.allow",
            category: "AI Providers",
            name: "Allow OpenAI",
            description: "Enable API access to OpenAI (api.openai.com).",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(false),
            enabled_by: None,
            metadata: SettingMetadata {
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        post: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.openai.api_key",
            category: "AI Providers",
            name: "OpenAI API Key",
            description: "API key for OpenAI. Injected as OPENAI_API_KEY env var.",
            setting_type: SettingType::ApiKey,
            default_value: SettingValue::Text(String::new()),
            enabled_by: Some("ai.openai.allow"),
            metadata: SettingMetadata {
                env_vars: vec!["OPENAI_API_KEY".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.openai.domains",
            category: "AI Providers",
            name: "OpenAI Domains",
            description: "Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("*.openai.com".into()),
            enabled_by: Some("ai.openai.allow"),
            metadata: SettingMetadata::default(),
        },
        SettingDef {
            id: "ai.google.allow",
            category: "AI Providers",
            name: "Allow Google AI",
            description: "Enable API access to Google AI (*.googleapis.com).",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        post: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.google.api_key",
            category: "AI Providers",
            name: "Google AI API Key",
            description: "API key for Google AI. Injected as GEMINI_API_KEY env var.",
            setting_type: SettingType::ApiKey,
            default_value: SettingValue::Text(String::new()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata {
                env_vars: vec!["GEMINI_API_KEY".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "ai.google.domains",
            category: "AI Providers",
            name: "Google AI Domains",
            description: "Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("*.googleapis.com".into()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata::default(),
        },
        SettingDef {
            id: "ai.google.gemini.settings_json",
            category: "AI Providers",
            name: "Gemini settings.json",
            description: "Content for ~/.gemini/settings.json. Session retention, auth, MCP servers, etc.",
            setting_type: SettingType::File,
            default_value: SettingValue::Text(r#"{"homeDirectoryWarningDismissed":true,"approvalMode":"yolo","general":{"enableAutoUpdate":false,"enableAutoUpdateNotification":false,"sessionRetention":{"enabled":true,"maxAge":"30d","warningAcknowledged":true}},"ui":{"hideTips":true,"showHomeDirectoryWarning":false,"showCompatibilityWarnings":false,"showShortcutsHint":false},"privacy":{"usageStatisticsEnabled":false},"telemetry":{"enabled":false},"security":{"auth":{"selectedType":"gemini-api-key"},"folderTrust.enabled":false},"ide":{"hasSeenNudge":true},"tools":{"sandbox":false}}"#.into()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.gemini/settings.json".into()), ..Default::default() },
        },
        SettingDef {
            id: "ai.google.gemini.projects_json",
            category: "AI Providers",
            name: "Gemini projects.json",
            description: "Content for ~/.gemini/projects.json. Project directory mappings.",
            setting_type: SettingType::File,
            default_value: SettingValue::Text(r#"{"projects":{"/root":"root"}}"#.into()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.gemini/projects.json".into()), ..Default::default() },
        },
        SettingDef {
            id: "ai.google.gemini.trusted_folders_json",
            category: "AI Providers",
            name: "Gemini trustedFolders.json",
            description: "Content for ~/.gemini/trustedFolders.json. Pre-trusted workspace dirs.",
            setting_type: SettingType::File,
            default_value: SettingValue::Text(r#"{"/root":"TRUST_FOLDER"}"#.into()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.gemini/trustedFolders.json".into()), ..Default::default() },
        },
        SettingDef {
            id: "ai.google.gemini.installation_id",
            category: "AI Providers",
            name: "Gemini installation_id",
            description: "Content for ~/.gemini/installation_id. Stable UUID avoids first-run prompts.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("capsem-sandbox-00000000-0000-0000-0000-000000000000".into()),
            enabled_by: Some("ai.google.allow"),
            metadata: SettingMetadata { guest_path: Some("/root/.gemini/installation_id".into()), ..Default::default() },
        },
        // -- Package Registries --
        SettingDef {
            id: "registry.github.allow",
            category: "Package Registries",
            name: "Allow GitHub",
            description: "Enable access to GitHub and GitHub-hosted content.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec![
                    "github.com".into(),
                    "*.github.com".into(),
                    "*.githubusercontent.com".into(),
                ],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        post: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "registry.npm.allow",
            category: "Package Registries",
            name: "Allow npm",
            description: "Enable access to the npm package registry.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec!["registry.npmjs.org".into(), "*.npmjs.org".into()],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "registry.pypi.allow",
            category: "Package Registries",
            name: "Allow PyPI",
            description: "Enable access to the Python Package Index.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec!["pypi.org".into(), "files.pythonhosted.org".into()],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "registry.crates.allow",
            category: "Package Registries",
            name: "Allow crates.io",
            description: "Enable access to the Rust crate registry.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec!["crates.io".into(), "static.crates.io".into()],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "registry.debian.allow",
            category: "Package Registries",
            name: "Allow Debian repos",
            description: "Enable access to Debian package repositories.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec!["deb.debian.org".into(), "security.debian.org".into()],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        SettingDef {
            id: "registry.elie.allow",
            category: "Package Registries",
            name: "Allow elie.net",
            description: "Enable access to elie.net and subdomains.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata {
                domains: vec!["elie.net".into(), "*.elie.net".into()],
                rules: HashMap::from([(
                    "default".into(),
                    HttpMethodPermissions {
                        get: true,
                        post: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        },
        // -- Guest Environment --
        SettingDef {
            id: "guest.shell.term",
            category: "Guest Environment",
            name: "TERM",
            description: "Terminal type for the guest shell.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("xterm-256color".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                env_vars: vec!["TERM".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "guest.shell.home",
            category: "Guest Environment",
            name: "HOME",
            description: "Home directory for the guest shell.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("/root".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                env_vars: vec!["HOME".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "guest.shell.path",
            category: "Guest Environment",
            name: "PATH",
            description: "Executable search path for the guest shell.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                env_vars: vec!["PATH".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "guest.shell.lang",
            category: "Guest Environment",
            name: "LANG",
            description: "Locale for the guest shell.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("C".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                env_vars: vec!["LANG".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "guest.tls.ca_bundle",
            category: "Guest Environment",
            name: "CA bundle path",
            description: "Path to the CA certificate bundle in the guest. Injected as REQUESTS_CA_BUNDLE, NODE_EXTRA_CA_CERTS, and SSL_CERT_FILE.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("/etc/ssl/certs/ca-certificates.crt".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                env_vars: vec![
                    "REQUESTS_CA_BUNDLE".into(),
                    "NODE_EXTRA_CA_CERTS".into(),
                    "SSL_CERT_FILE".into(),
                ],
                ..Default::default()
            },
        },
        // -- Network --
        SettingDef {
            id: "network.default_action",
            category: "Network",
            name: "Default action",
            description: "Action for domains not in any allow/block list.",
            setting_type: SettingType::Text,
            default_value: SettingValue::Text("deny".into()),
            enabled_by: None,
            metadata: SettingMetadata {
                choices: vec!["allow".into(), "deny".into()],
                ..Default::default()
            },
        },
        SettingDef {
            id: "network.log_bodies",
            category: "Network",
            name: "Log request bodies",
            description: "Capture request/response bodies in telemetry.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(false),
            enabled_by: None,
            metadata: SettingMetadata::default(),
        },
        SettingDef {
            id: "network.max_body_capture",
            category: "Network",
            name: "Max body capture",
            description: "Maximum bytes of body to capture in telemetry.",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(4096),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(0),
                max: Some(1_048_576),
                ..Default::default()
            },
        },
        // -- Session --
        SettingDef {
            id: "session.retention_days",
            category: "Session",
            name: "Session retention",
            description: "Number of days to retain session data.",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(30),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(1),
                max: Some(365),
                ..Default::default()
            },
        },
        // -- Appearance --
        SettingDef {
            id: "appearance.dark_mode",
            category: "Appearance",
            name: "Dark mode",
            description: "Use dark color scheme in the UI.",
            setting_type: SettingType::Bool,
            default_value: SettingValue::Bool(true),
            enabled_by: None,
            metadata: SettingMetadata::default(),
        },
        SettingDef {
            id: "appearance.font_size",
            category: "Appearance",
            name: "Font size",
            description: "Terminal font size in pixels.",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(14),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(8),
                max: Some(32),
                ..Default::default()
            },
        },
        // -- VM --
        SettingDef {
            id: "vm.scratch_disk_size_gb",
            category: "VM",
            name: "Scratch disk size",
            description: "Size of the ephemeral scratch disk in GB.",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(8),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(1),
                max: Some(128),
                ..Default::default()
            },
        },
        // -- Session (continued) --
        SettingDef {
            id: "session.max_sessions",
            category: "Session",
            name: "Maximum sessions",
            description: "Keep at most this many sessions (oldest culled first).",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(100),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(1),
                max: Some(10000),
                ..Default::default()
            },
        },
        SettingDef {
            id: "session.max_disk_gb",
            category: "Session",
            name: "Maximum disk usage",
            description: "Maximum total disk usage for all sessions in GB.",
            setting_type: SettingType::Number,
            default_value: SettingValue::Number(100),
            enabled_by: None,
            metadata: SettingMetadata {
                min: Some(1),
                max: Some(1000),
                ..Default::default()
            },
        },
    ]
}

/// Returns an empty settings file (all defaults).
pub fn default_settings_file() -> SettingsFile {
    SettingsFile::default()
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

/// User config path: ~/.capsem/user.toml
pub fn user_config_path() -> Option<std::path::PathBuf> {
    dirs_path("HOME").map(|h| h.join(".capsem").join("user.toml"))
}

/// Corporate config path: /etc/capsem/corp.toml
pub fn corp_config_path() -> std::path::PathBuf {
    std::path::PathBuf::from("/etc/capsem/corp.toml")
}

fn dirs_path(env_var: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var).ok().map(std::path::PathBuf::from)
}

/// Load a settings file from disk. Returns empty SettingsFile if file missing.
pub fn load_settings_file(path: &Path) -> Result<SettingsFile, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|e| format!("failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SettingsFile::default()),
        Err(e) => Err(format!("failed to read {}: {}", path.display(), e)),
    }
}

/// Write a settings file to disk as TOML. Creates parent dirs if needed.
pub fn write_settings_file(path: &Path, file: &SettingsFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create dir {}: {}", parent.display(), e))?;
    }
    let content = toml::to_string_pretty(file)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    std::fs::write(path, content)
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// Load both settings files from standard locations.
pub fn load_settings_files() -> (SettingsFile, SettingsFile) {
    let user = match user_config_path() {
        Some(path) => load_settings_file(&path).unwrap_or_else(|e| {
            tracing::warn!("user settings: {e}");
            SettingsFile::default()
        }),
        None => SettingsFile::default(),
    };

    let corp = load_settings_file(&corp_config_path()).unwrap_or_else(|e| {
        tracing::warn!("corp settings: {e}");
        SettingsFile::default()
    });

    (user, corp)
}

/// Write user settings to ~/.capsem/user.toml.
pub fn write_user_settings(file: &SettingsFile) -> Result<(), String> {
    let path = user_config_path().ok_or("HOME not set")?;
    write_settings_file(&path, file)
}

/// Whether the current process can write corp settings (always false).
pub fn can_write_corp_settings() -> bool {
    false
}

/// Validate a setting value before persisting.
///
/// For `File`-type settings whose `guest_path` ends in `.json`, the value
/// must be valid JSON (or empty). Other types pass through without validation.
pub fn validate_setting_value(id: &str, value: &SettingValue) -> Result<(), String> {
    let defs = setting_definitions();
    let def = match defs.iter().find(|d| d.id == id) {
        Some(d) => d,
        None => return Ok(()), // dynamic / unknown settings pass through
    };

    if def.setting_type != SettingType::File {
        return Ok(());
    }

    let text = match value.as_text() {
        Some(t) => t,
        None => return Ok(()), // non-text value for a File setting is odd but not our problem here
    };

    if text.is_empty() {
        return Ok(()); // empty means "use default" or "don't inject"
    }

    // JSON validation for .json guest paths
    if let Some(path) = &def.metadata.guest_path {
        if path.ends_with(".json") {
            serde_json::from_str::<serde_json::Value>(text)
                .map_err(|e| format!("invalid JSON for {id}: {e}"))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Merge / resolve
// ---------------------------------------------------------------------------

/// Check if a setting is locked by corp.
pub fn is_setting_corp_locked(id: &str, corp: &SettingsFile) -> bool {
    corp.settings.contains_key(id)
}

/// Resolve all settings from user + corp files against the registry.
///
/// For each registered definition + any dynamic keys (guest.env.*),
/// corp overrides user, user overrides default.
/// Computes `enabled` from parent toggle.
pub fn resolve_settings(user: &SettingsFile, corp: &SettingsFile) -> Vec<ResolvedSetting> {
    let defs = setting_definitions();
    let mut resolved = Vec::new();

    for def in &defs {
        let (effective_value, source, modified) = resolve_value(def.id, &def.default_value, user, corp);
        let corp_locked = corp.settings.contains_key(def.id);

        resolved.push(ResolvedSetting {
            id: def.id.to_string(),
            category: def.category.to_string(),
            name: def.name.to_string(),
            description: def.description.to_string(),
            setting_type: def.setting_type,
            default_value: def.default_value.clone(),
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: def.enabled_by.map(String::from),
            enabled: true, // computed below
            metadata: def.metadata.clone(),
        });
    }

    // Dynamic settings: guest.env.* (not in registry)
    let dynamic_keys = collect_dynamic_keys(user, corp);
    for key in dynamic_keys {
        let default = SettingValue::Text(String::new());
        let (effective_value, source, modified) = resolve_value(&key, &default, user, corp);
        let corp_locked = corp.settings.contains_key(&key);

        resolved.push(ResolvedSetting {
            id: key.clone(),
            category: "Guest Environment".to_string(),
            name: key.strip_prefix("guest.env.").unwrap_or(&key).to_string(),
            description: format!("Guest environment variable: {}", key.strip_prefix("guest.env.").unwrap_or(&key)),
            setting_type: SettingType::Text,
            default_value: default,
            effective_value,
            source,
            modified,
            corp_locked,
            enabled_by: None,
            enabled: true,
            metadata: SettingMetadata::default(),
        });
    }

    // Compute enabled_by: look up parent toggle value
    compute_enabled(&mut resolved);

    resolved
}

/// Resolve a single setting value: corp > user > default.
fn resolve_value(
    id: &str,
    default: &SettingValue,
    user: &SettingsFile,
    corp: &SettingsFile,
) -> (SettingValue, PolicySource, Option<String>) {
    if let Some(entry) = corp.settings.get(id) {
        (entry.value.clone(), PolicySource::Corp, Some(entry.modified.clone()))
    } else if let Some(entry) = user.settings.get(id) {
        (entry.value.clone(), PolicySource::User, Some(entry.modified.clone()))
    } else {
        (default.clone(), PolicySource::Default, None)
    }
}

/// Collect all dynamic keys (guest.env.*) from both files.
fn collect_dynamic_keys(user: &SettingsFile, corp: &SettingsFile) -> Vec<String> {
    let mut keys: Vec<String> = user
        .settings
        .keys()
        .chain(corp.settings.keys())
        .filter(|k| k.starts_with("guest.env."))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Compute the `enabled` flag for each setting based on its parent toggle.
fn compute_enabled(settings: &mut [ResolvedSetting]) {
    // Build a lookup of id -> effective bool value
    let values: HashMap<String, bool> = settings
        .iter()
        .filter_map(|s| s.effective_value.as_bool().map(|b| (s.id.clone(), b)))
        .collect();

    for s in settings.iter_mut() {
        if let Some(ref parent_id) = s.enabled_by {
            s.enabled = values.get(parent_id.as_str()).copied().unwrap_or(false);
        }
        // else enabled stays true (set during construction)
    }
}

// ---------------------------------------------------------------------------
// Translation: settings -> policy objects
// ---------------------------------------------------------------------------

/// Parse a comma-separated domain list into trimmed individual entries.
fn parse_domain_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

/// Check if a candidate domain matches any corp-blocked pattern.
/// Uses the same wildcard logic as DomainPattern: suffix match for `*.foo.com`,
/// exact match otherwise.
fn corp_blocked_matches(candidate: &str, corp_blocked: &[String]) -> bool {
    let candidate = candidate.to_lowercase();
    for pattern in corp_blocked {
        let pattern = pattern.to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            if candidate.ends_with(&format!(".{suffix}")) || candidate == suffix {
                return true;
            }
        } else if candidate == pattern {
            return true;
        }
    }
    false
}

/// Build a DomainPolicy from resolved settings.
///
/// - Bool toggles with domain metadata (registries) -> allow/block those domains
/// - `.domains` Text settings -> allow/block parsed domain patterns
/// - Corp-locked-off services use UNION of default + effective domains for blocking
/// - Default action from network.default_action
pub fn settings_to_domain_policy(resolved: &[ResolvedSetting]) -> DomainPolicy {
    let mut allow_list: Vec<String> = Vec::new();
    let mut block_list: Vec<String> = Vec::new();

    // Existing: Bool toggles with domain metadata (registries)
    for s in resolved {
        if s.metadata.domains.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if enabled {
            allow_list.extend(s.metadata.domains.clone());
        } else {
            block_list.extend(s.metadata.domains.clone());
        }
    }

    // Pass 1: collect corp-blocked domain patterns from .domains settings.
    // When corp locks .allow to false, use UNION of default + effective so
    // user can't shrink the block list below defaults.
    let mut corp_blocked: Vec<String> = Vec::new();
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            let defaults = parse_domain_list(s.default_value.as_text().unwrap_or(""));
            let effective = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
            let mut all: Vec<String> = defaults;
            for d in effective {
                if !all.contains(&d) {
                    all.push(d);
                }
            }
            block_list.extend(all.clone());
            corp_blocked.extend(all);
        }
    }

    // Pass 2: process non-corp-locked .domains settings
    for s in resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            continue; // Already handled in pass 1
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        if toggle_on {
            // Filter: don't allow domains that corp has blocked
            for d in domains {
                if corp_blocked_matches(&d, &corp_blocked) {
                    block_list.push(d); // Override: corp says no
                } else {
                    allow_list.push(d);
                }
            }
        } else {
            block_list.extend(domains);
        }
    }

    let default_action = resolved
        .iter()
        .find(|s| s.id == "network.default_action")
        .and_then(|s| s.effective_value.as_text())
        .and_then(|s| match s {
            "allow" => Some(Action::Allow),
            "deny" => Some(Action::Deny),
            _ => None,
        })
        .unwrap_or(Action::Deny);

    DomainPolicy::new(&allow_list, &block_list, default_action)
}

/// Build an HttpPolicy from resolved settings.
///
/// Generates HttpRules from setting metadata.rules for enabled toggles.
pub fn settings_to_http_policy(resolved: &[ResolvedSetting]) -> HttpPolicy {
    let domain_policy = settings_to_domain_policy(resolved);

    let mut http_rules: Vec<HttpRule> = Vec::new();

    for s in resolved {
        if s.metadata.rules.is_empty() {
            continue;
        }
        if s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        if !enabled {
            continue;
        }

        // For each rule in metadata, generate HttpRules for the setting's domains
        let rule_domains: Vec<&str> = s.metadata.domains.iter().map(|d| d.as_str()).collect();

        for (_rule_name, perms) in &s.metadata.rules {
            let domains_for_rule = if perms.domains.is_empty() {
                rule_domains.clone()
            } else {
                perms.domains.iter().map(|d| d.as_str()).collect()
            };

            let path_pattern = perms.path.as_deref().unwrap_or("*").to_string();

            for domain in &domains_for_rule {
                // Skip wildcard domains for HTTP rules (they apply at domain level only)
                if domain.starts_with("*.") {
                    continue;
                }
                // Generate allow rules for each enabled method
                for (method, allowed) in [
                    ("GET", perms.get),
                    ("POST", perms.post),
                    ("PUT", perms.put),
                    ("DELETE", perms.delete),
                ] {
                    if allowed {
                        http_rules.push(HttpRule {
                            domain: domain.to_lowercase(),
                            method: method.to_string(),
                            path_pattern: path_pattern.clone(),
                            action: Action::Allow,
                        });
                    }
                }
            }
        }
    }

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "network.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(false);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "network.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    HttpPolicy::new(domain_policy, http_rules, log_bodies, max_body_capture)
}

/// Extract guest config from resolved settings.
///
/// Dynamic keys with prefix `guest.env.` become environment variables.
/// AI provider API keys and boot files are always injected when the key/value
/// is non-empty, regardless of the provider toggle. The toggle controls network
/// access (domain policy), not whether credentials are available in the VM.
/// This ensures the user can enable a provider at runtime without rebooting.
pub fn settings_to_guest_config(resolved: &[ResolvedSetting]) -> GuestConfig {
    use capsem_proto::{validate_env_key, validate_env_value, validate_file_path};

    let mut env = HashMap::new();
    let mut files = Vec::new();

    for s in resolved {
        let text_value = s.effective_value.as_text().unwrap_or("");

        // Provider allow toggles: inject CAPSEM_<PROVIDER>_ALLOWED=1|0
        // so the guest banner can show which AI tools are enabled.
        if s.setting_type == SettingType::Bool {
            let provider_env = match s.id.as_str() {
                "ai.anthropic.allow" => Some("CAPSEM_ANTHROPIC_ALLOWED"),
                "ai.openai.allow" => Some("CAPSEM_OPENAI_ALLOWED"),
                "ai.google.allow" => Some("CAPSEM_GOOGLE_ALLOWED"),
                _ => None,
            };
            if let Some(var_name) = provider_env {
                let val = if s.effective_value.as_bool().unwrap_or(false) { "1" } else { "0" };
                env.insert(var_name.to_string(), val.to_string());
            }
        }

        // Metadata-driven env var injection: if the setting declares env_vars
        // and the effective value is non-empty text, inject each env var.
        if !s.metadata.env_vars.is_empty() && !text_value.is_empty() {
            for var_name in &s.metadata.env_vars {
                if let Err(e) = validate_env_key(var_name) {
                    tracing::warn!("skipping invalid env var from metadata: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(text_value) {
                    tracing::warn!("skipping env var {var_name}: invalid value: {e}");
                    continue;
                }
                env.insert(var_name.clone(), text_value.to_string());
            }
        }

        // Boot files: File-type or Text-type settings with a guest_path.
        // Always inject if non-empty -- the allow toggle controls network
        // policy, not file availability.
        if (s.setting_type == SettingType::File || s.setting_type == SettingType::Text)
            && !text_value.is_empty()
        {
            if let Some(ref guest_path) = s.metadata.guest_path {
                if let Err(e) = validate_file_path(guest_path) {
                    tracing::warn!("skipping boot file: {e}");
                    continue;
                }
                files.push(GuestFile {
                    path: guest_path.clone(),
                    content: text_value.to_string(),
                    mode: 0o644,
                });
            }
        }

        // Dynamic guest.env.* settings (not in registry)
        if let Some(var_name) = s.id.strip_prefix("guest.env.") {
            if !text_value.is_empty() {
                if let Err(e) = validate_env_key(var_name) {
                    tracing::warn!("skipping dynamic env var: {e}");
                    continue;
                }
                if let Err(e) = validate_env_value(text_value) {
                    tracing::warn!("skipping dynamic env var {var_name}: invalid value: {e}");
                    continue;
                }
                env.insert(var_name.to_string(), text_value.to_string());
            }
        }
    }

    GuestConfig {
        env: if env.is_empty() { None } else { Some(env) },
        files: if files.is_empty() { None } else { Some(files) },
    }
}

/// Extract VM settings from resolved settings.
pub fn settings_to_vm_settings(resolved: &[ResolvedSetting]) -> VmSettings {
    let scratch_disk_size_gb = resolved
        .iter()
        .find(|s| s.id == "vm.scratch_disk_size_gb")
        .and_then(|s| s.effective_value.as_number())
        .map(|n| n as u32);

    VmSettings {
        scratch_disk_size_gb: Some(scratch_disk_size_gb.unwrap_or(8)),
    }
}

// ---------------------------------------------------------------------------
// High-level entry points
// ---------------------------------------------------------------------------

/// Load and merge settings, then build an HttpPolicy.
pub fn load_merged_policy() -> HttpPolicy {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_http_policy(&resolved)
}

/// Build a `NetworkPolicy` (new policy engine) from merged settings.
///
/// Bridges settings into per-domain read/write rules:
/// - Disabled toggles with domains get read=false, write=false
/// - Enabled toggles with domains get read=true, write=true
/// - Default action maps to default_allow_read and default_allow_write
pub fn load_merged_network_policy() -> super::policy::NetworkPolicy {
    use super::policy::{DomainMatcher, NetworkPolicy, PolicyRule};

    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);

    let mut rules = Vec::new();

    // Build rules from settings with domain metadata (registries)
    for s in &resolved {
        if s.metadata.domains.is_empty() || s.setting_type != SettingType::Bool {
            continue;
        }
        let enabled = s.effective_value.as_bool().unwrap_or(false);
        for domain in &s.metadata.domains {
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    // Build rules from .domains text settings (AI providers)
    // Corp block enforcement: same two-pass approach as settings_to_domain_policy
    let mut corp_blocked: Vec<String> = Vec::new();
    for s in &resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            let defaults = parse_domain_list(s.default_value.as_text().unwrap_or(""));
            let effective = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
            let mut all: Vec<String> = defaults;
            for d in effective {
                if !all.contains(&d) {
                    all.push(d);
                }
            }
            for domain in &all {
                rules.push(PolicyRule {
                    matcher: DomainMatcher::parse(domain),
                    allow_read: false,
                    allow_write: false,
                });
            }
            corp_blocked.extend(all);
        }
    }
    for s in &resolved {
        if !s.id.ends_with(".domains") || s.setting_type != SettingType::Text {
            continue;
        }
        let toggle_id = s.id.replace(".domains", ".allow");
        let toggle = resolved.iter().find(|t| t.id == toggle_id);
        let corp_locked_off = match toggle {
            Some(t) => t.corp_locked && !t.effective_value.as_bool().unwrap_or(false),
            None => false,
        };
        if corp_locked_off {
            continue;
        }
        let toggle_on = toggle
            .and_then(|t| t.effective_value.as_bool())
            .unwrap_or(false);
        let domains = parse_domain_list(s.effective_value.as_text().unwrap_or(""));
        for domain in &domains {
            let blocked = corp_blocked_matches(domain, &corp_blocked);
            let enabled = toggle_on && !blocked;
            rules.push(PolicyRule {
                matcher: DomainMatcher::parse(domain),
                allow_read: enabled,
                allow_write: enabled,
            });
        }
    }

    let default_action = resolved
        .iter()
        .find(|s| s.id == "network.default_action")
        .and_then(|s| s.effective_value.as_text())
        .map(|s| s == "allow")
        .unwrap_or(false);

    let log_bodies = resolved
        .iter()
        .find(|s| s.id == "network.log_bodies")
        .and_then(|s| s.effective_value.as_bool())
        .unwrap_or(true);

    let max_body_capture = resolved
        .iter()
        .find(|s| s.id == "network.max_body_capture")
        .and_then(|s| s.effective_value.as_number())
        .unwrap_or(4096) as usize;

    let mut policy = NetworkPolicy::new(rules, default_action, default_action);
    policy.log_bodies = log_bodies;
    policy.max_body_capture = max_body_capture;
    policy
}

/// Load and merge guest config from standard locations.
pub fn load_merged_guest_config() -> GuestConfig {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_guest_config(&resolved)
}

/// Load and merge VM settings from standard locations.
pub fn load_merged_vm_settings() -> VmSettings {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    settings_to_vm_settings(&resolved)
}

/// Load all resolved settings (for UI).
pub fn load_merged_settings() -> Vec<ResolvedSetting> {
    let (user, corp) = load_settings_files();
    resolve_settings(&user, &corp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_file() -> SettingsFile {
        SettingsFile::default()
    }

    fn now_str() -> String {
        "2026-02-25T00:00:00Z".to_string()
    }

    fn file_with(entries: Vec<(&str, SettingValue)>) -> SettingsFile {
        let mut settings = HashMap::new();
        for (id, value) in entries {
            settings.insert(id.to_string(), SettingEntry {
                value,
                modified: now_str(),
            });
        }
        SettingsFile { settings }
    }

    // -----------------------------------------------------------------------
    // A: Corp override (7)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_override_bool() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_text() {
        let user = file_with(vec![("network.default_action", SettingValue::Text("allow".into()))]);
        let corp = file_with(vec![("network.default_action", SettingValue::Text("deny".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("deny".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_number() {
        let user = file_with(vec![("network.max_body_capture", SettingValue::Number(8192))]);
        let corp = file_with(vec![("network.max_body_capture", SettingValue::Number(1024))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.max_body_capture").unwrap();
        assert_eq!(s.effective_value, SettingValue::Number(1024));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_api_key() {
        let user = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.anthropic.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_guest_env() {
        let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
        let corp = file_with(vec![("guest.env.EDITOR", SettingValue::Text("nano".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "guest.env.EDITOR").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("nano".into()));
        assert_eq!(s.source, PolicySource::Corp);
    }

    #[test]
    fn corp_override_mixed_categories() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.log_bodies", SettingValue::Bool(true)),
            ("appearance.dark_mode", SettingValue::Bool(false)),
        ]);
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("network.log_bodies", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);

        let ai = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(ai.effective_value, SettingValue::Bool(false));
        assert_eq!(ai.source, PolicySource::Corp);

        let log = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(log.effective_value, SettingValue::Bool(false));
        assert_eq!(log.source, PolicySource::Corp);

        // appearance.dark_mode not in corp -> user value
        let dark = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
        assert_eq!(dark.effective_value, SettingValue::Bool(false));
        assert_eq!(dark.source, PolicySource::User);
    }

    #[test]
    fn corp_overrides_all_registry_toggles() {
        let corp = file_with(vec![
            ("registry.github.allow", SettingValue::Bool(false)),
            ("registry.npm.allow", SettingValue::Bool(false)),
            ("registry.pypi.allow", SettingValue::Bool(false)),
            ("registry.crates.allow", SettingValue::Bool(false)),
            ("registry.debian.allow", SettingValue::Bool(false)),
            ("registry.elie.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&empty_file(), &corp);
        for s in &resolved {
            if s.id.starts_with("registry.") && s.id.ends_with(".allow") {
                assert_eq!(s.effective_value, SettingValue::Bool(false), "failed for {}", s.id);
                assert_eq!(s.source, PolicySource::Corp);
            }
        }
    }

    // -----------------------------------------------------------------------
    // B: User cannot expand (3)
    // -----------------------------------------------------------------------

    #[test]
    fn user_cannot_enable_blocked_provider() {
        // Corp blocks anthropic, user tries to enable
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(false));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_change_corp_default_action() {
        let user = file_with(vec![("network.default_action", SettingValue::Text("allow".into()))]);
        let corp = file_with(vec![("network.default_action", SettingValue::Text("deny".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("deny".into()));
        assert!(s.corp_locked);
    }

    #[test]
    fn user_cannot_override_corp_api_key() {
        let user = file_with(vec![("ai.openai.api_key", SettingValue::Text("user-key".into()))]);
        let corp = file_with(vec![("ai.openai.api_key", SettingValue::Text("corp-key".into()))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert_eq!(s.effective_value, SettingValue::Text("corp-key".into()));
        assert!(s.corp_locked);
    }

    // -----------------------------------------------------------------------
    // C: User isolation (4)
    // -----------------------------------------------------------------------

    #[test]
    fn can_write_corp_is_always_false() {
        assert!(!can_write_corp_settings());
    }

    #[test]
    fn write_user_settings_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_user.toml");
        let file = file_with(vec![("network.log_bodies", SettingValue::Bool(true))]);
        write_settings_file(&path, &file).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_user_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.toml");
        let file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.max_body_capture", SettingValue::Number(8192)),
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        write_settings_file(&path, &file).unwrap();
        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(file.settings.len(), loaded.settings.len());
        for (key, entry) in &file.settings {
            let loaded_entry = loaded.settings.get(key).unwrap();
            assert_eq!(entry.value, loaded_entry.value, "mismatch for {key}");
        }
    }

    #[test]
    fn write_user_settings_preserves_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preserve.toml");
        let mut file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.log_bodies", SettingValue::Bool(false)),
        ]);
        write_settings_file(&path, &file).unwrap();

        // Update one setting
        file.settings.get_mut("network.log_bodies").unwrap().value = SettingValue::Bool(true);
        write_settings_file(&path, &file).unwrap();

        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(
            loaded.settings.get("ai.anthropic.allow").unwrap().value,
            SettingValue::Bool(true),
        );
        assert_eq!(
            loaded.settings.get("network.log_bodies").unwrap().value,
            SettingValue::Bool(true),
        );
    }

    // -----------------------------------------------------------------------
    // D: Defaults (5)
    // -----------------------------------------------------------------------

    #[test]
    fn default_settings_file_is_empty() {
        let file = default_settings_file();
        assert!(file.settings.is_empty());
    }

    #[test]
    fn default_resolve_has_all_definitions() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let defs = setting_definitions();
        for def in &defs {
            assert!(
                resolved.iter().any(|s| s.id == def.id),
                "missing definition: {}",
                def.id,
            );
        }
    }

    #[test]
    fn default_ai_providers_blocked() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &["ai.anthropic.allow", "ai.openai.allow"] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(false), "expected {id} to be false");
        }
    }

    #[test]
    fn default_google_ai_allowed() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.google.allow").unwrap();
        assert_eq!(s.effective_value, SettingValue::Bool(true), "expected ai.google.allow to be true");
    }

    #[test]
    fn default_registries_allowed() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for id in &[
            "registry.github.allow",
            "registry.npm.allow",
            "registry.pypi.allow",
            "registry.crates.allow",
            "registry.debian.allow",
            "registry.elie.allow",
        ] {
            let s = resolved.iter().find(|s| s.id == *id).unwrap();
            assert_eq!(s.effective_value, SettingValue::Bool(true), "expected {id} to be true");
        }
    }

    #[test]
    fn default_network_session_appearance() {
        let resolved = resolve_settings(&empty_file(), &empty_file());

        let da = resolved.iter().find(|s| s.id == "network.default_action").unwrap();
        assert_eq!(da.effective_value, SettingValue::Text("deny".into()));

        let lb = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(lb.effective_value, SettingValue::Bool(false));

        let mbc = resolved.iter().find(|s| s.id == "network.max_body_capture").unwrap();
        assert_eq!(mbc.effective_value, SettingValue::Number(4096));

        let rd = resolved.iter().find(|s| s.id == "session.retention_days").unwrap();
        assert_eq!(rd.effective_value, SettingValue::Number(30));

        let dm = resolved.iter().find(|s| s.id == "appearance.dark_mode").unwrap();
        assert_eq!(dm.effective_value, SettingValue::Bool(true));

        let fs = resolved.iter().find(|s| s.id == "appearance.font_size").unwrap();
        assert_eq!(fs.effective_value, SettingValue::Number(14));
    }

    // -----------------------------------------------------------------------
    // E: Definitions (4)
    // -----------------------------------------------------------------------

    #[test]
    fn definitions_have_unique_ids() {
        let defs = setting_definitions();
        let mut ids: Vec<&str> = defs.iter().map(|d| d.id).collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "duplicate setting IDs found");
    }

    #[test]
    fn definitions_have_nonempty_descriptions() {
        for def in setting_definitions() {
            assert!(!def.description.is_empty(), "empty description for {}", def.id);
            assert!(!def.name.is_empty(), "empty name for {}", def.id);
        }
    }

    #[test]
    fn registry_toggles_have_domain_metadata() {
        let defs = setting_definitions();
        for def in &defs {
            if def.id.starts_with("registry.") && def.id.ends_with(".allow") {
                assert!(
                    !def.metadata.domains.is_empty(),
                    "toggle {} has no domain metadata",
                    def.id,
                );
            }
        }
    }

    #[test]
    fn ai_providers_have_domains_settings() {
        let defs = setting_definitions();
        for prefix in &["ai.anthropic", "ai.openai", "ai.google"] {
            let domains_id = format!("{prefix}.domains");
            let def = defs.iter().find(|d| d.id == domains_id);
            assert!(def.is_some(), "missing {domains_id} setting");
            let def = def.unwrap();
            assert_eq!(def.setting_type, SettingType::Text);
            assert!(def.enabled_by.is_some());
        }
    }

    #[test]
    fn choice_settings_have_choices_metadata() {
        let defs = setting_definitions();
        let da = defs.iter().find(|d| d.id == "network.default_action").unwrap();
        assert!(!da.metadata.choices.is_empty());
        assert!(da.metadata.choices.contains(&"allow".to_string()));
        assert!(da.metadata.choices.contains(&"deny".to_string()));
    }

    // -----------------------------------------------------------------------
    // F: Source tracking (6)
    // -----------------------------------------------------------------------

    #[test]
    fn source_default() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Default);
        assert!(s.modified.is_none());
    }

    #[test]
    fn source_user() {
        let user = file_with(vec![("network.log_bodies", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::User);
        assert!(s.modified.is_some());
    }

    #[test]
    fn source_corp() {
        let corp = file_with(vec![("network.log_bodies", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let s = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Corp);
        assert!(s.modified.is_some());
    }

    #[test]
    fn source_corp_beats_user() {
        let user = file_with(vec![("network.log_bodies", SettingValue::Bool(true))]);
        let corp = file_with(vec![("network.log_bodies", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &corp);
        let s = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert_eq!(s.source, PolicySource::Corp);
        assert_eq!(s.effective_value, SettingValue::Bool(false));
    }

    #[test]
    fn source_dynamic_guest_env() {
        let user = file_with(vec![("guest.env.FOO", SettingValue::Text("bar".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "guest.env.FOO").unwrap();
        assert_eq!(s.source, PolicySource::User);
        assert_eq!(s.category, "Guest Environment");
    }

    #[test]
    fn is_setting_corp_locked_test() {
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        assert!(is_setting_corp_locked("ai.anthropic.allow", &corp));
        assert!(!is_setting_corp_locked("ai.openai.allow", &corp));
    }

    // -----------------------------------------------------------------------
    // G: enabled_by (4)
    // -----------------------------------------------------------------------

    #[test]
    fn enabled_by_parent_on_child_enabled() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert!(child.enabled);
        assert_eq!(child.enabled_by, Some("ai.anthropic.allow".to_string()));
    }

    #[test]
    fn enabled_by_parent_off_child_disabled() {
        // Default: ai.anthropic.allow is false
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let child = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert!(!child.enabled);
    }

    #[test]
    fn enabled_by_none_always_enabled() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "network.log_bodies").unwrap();
        assert!(s.enabled);
        assert!(s.enabled_by.is_none());
    }

    #[test]
    fn enabled_by_chain_not_supported() {
        // Only one level of enabled_by is supported.
        // A child with enabled_by pointing to a non-existent parent is disabled.
        let mut user = empty_file();
        // Simulate a setting with enabled_by pointing to a non-bool setting
        // This should result in enabled=false since the parent can't be resolved to bool
        let resolved = resolve_settings(&user, &empty_file());

        // All api_key settings have enabled_by pointing to .allow toggles
        // When the toggle is off (default for AI), api_key is disabled
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(!key.enabled);

        // Turn on the toggle -> key is enabled
        user = file_with(vec![("ai.openai.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let key = resolved.iter().find(|s| s.id == "ai.openai.api_key").unwrap();
        assert!(key.enabled);
    }

    // -----------------------------------------------------------------------
    // H: Translation (5)
    // -----------------------------------------------------------------------

    #[test]
    fn settings_to_domain_policy_defaults() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        // Registries enabled by default -> domains allowed
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("pypi.org");
        assert_eq!(action, Action::Allow);

        // Anthropic/OpenAI disabled by default -> domains blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);

        // Google AI enabled by default -> domains allowed
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);

        // Unknown domains denied
        let (action, _) = dp.evaluate("example.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_off_registry() {
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn settings_to_domain_policy_toggle_on_provider() {
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn settings_to_guest_config_from_dynamic() {
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
            ("guest.env.TERM", SettingValue::Text("xterm".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        assert_eq!(env.get("TERM").unwrap(), "xterm");
    }

    #[test]
    fn settings_to_http_policy_from_metadata_rules() {
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &empty_file());
        let hp = settings_to_http_policy(&resolved);

        // github.com is allowed at domain level
        let d = hp.evaluate_domain("github.com");
        assert_eq!(d.action, Action::Allow);

        // GET should be allowed (from metadata rules)
        let d = hp.evaluate_request("github.com", "GET", "/repos/foo");
        assert_eq!(d.action, Action::Allow);
    }

    // -----------------------------------------------------------------------
    // I: Roundtrip + edge cases (4)
    // -----------------------------------------------------------------------

    #[test]
    fn settings_file_toml_roundtrip() {
        let file = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.max_body_capture", SettingValue::Number(8192)),
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: SettingsFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(file.settings.len(), parsed.settings.len());
        for (key, entry) in &file.settings {
            assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
        }
    }

    #[test]
    fn settings_file_disk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk_roundtrip.toml");
        let file = file_with(vec![
            ("registry.github.allow", SettingValue::Bool(true)),
            ("appearance.font_size", SettingValue::Number(16)),
        ]);
        write_settings_file(&path, &file).unwrap();
        let loaded = load_settings_file(&path).unwrap();
        assert_eq!(file, loaded);
    }

    #[test]
    fn empty_files_use_defaults() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        for s in &resolved {
            assert_eq!(s.source, PolicySource::Default, "non-default source for {}", s.id);
        }
    }

    #[test]
    fn invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "{{{{not valid").unwrap();
        let result = load_settings_file(&path);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // TOML parsing from raw strings (M)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_real_user_toml_format() {
        // This is the exact format a real user.toml has on disk.
        let toml_str = r#"
[settings]
"ai.google.api_key" = { value = "AIzaSyTest1234", modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.allow" = { value = true, modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-test-key", modified = "2026-02-25T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse real user.toml format");
        assert_eq!(file.settings.len(), 3);
        assert_eq!(
            file.settings["ai.google.api_key"].value,
            SettingValue::Text("AIzaSyTest1234".into()),
        );
        assert_eq!(
            file.settings["ai.anthropic.allow"].value,
            SettingValue::Bool(true),
        );
        assert_eq!(
            file.settings["ai.anthropic.api_key"].value,
            SettingValue::Text("sk-ant-test-key".into()),
        );
    }

    #[test]
    fn parse_toml_mixed_value_types() {
        let toml_str = r#"
[settings]
"network.log_bodies" = { value = true, modified = "2026-01-01T00:00:00Z" }
"network.max_body_capture" = { value = 8192, modified = "2026-01-01T00:00:00Z" }
"network.default_action" = { value = "deny", modified = "2026-01-01T00:00:00Z" }
"appearance.font_size" = { value = 16, modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse mixed types");
        assert_eq!(file.settings["network.log_bodies"].value, SettingValue::Bool(true));
        assert_eq!(file.settings["network.max_body_capture"].value, SettingValue::Number(8192));
        assert_eq!(file.settings["network.default_action"].value, SettingValue::Text("deny".into()));
        assert_eq!(file.settings["appearance.font_size"].value, SettingValue::Number(16));
    }

    #[test]
    fn parse_toml_empty_settings_table() {
        let toml_str = "[settings]\n";
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse empty table");
        assert!(file.settings.is_empty());
    }

    #[test]
    fn parse_toml_completely_empty() {
        let file: SettingsFile = toml::from_str("").expect("should parse empty string");
        assert!(file.settings.is_empty());
    }

    #[test]
    fn parse_toml_missing_modified_fails() {
        // SettingEntry requires both value and modified
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "missing 'modified' field should fail");
    }

    #[test]
    fn parse_toml_missing_value_fails() {
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "missing 'value' field should fail");
    }

    #[test]
    fn parse_toml_extra_fields_ignored() {
        // TOML with extra unknown fields in the entry should still parse
        // (serde default behavior: ignore unknown fields)
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z", extra = "ignored" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        // By default serde does NOT deny unknown fields, so this should succeed.
        // If it fails, SettingEntry is using deny_unknown_fields.
        assert!(result.is_ok(), "extra fields should be ignored: {:?}", result.err());
    }

    #[test]
    fn parse_toml_wrong_value_type_fails() {
        // value is an array -- not a valid SettingValue variant
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = [1, 2, 3], modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "array value should fail deserialization");
    }

    #[test]
    fn parse_toml_unquoted_dotted_keys() {
        // In TOML, unquoted dotted keys create nested tables, not flat keys.
        // This is a common mistake: ai.anthropic.allow = { ... } creates
        // [ai] -> [anthropic] -> allow = { ... }, NOT a flat key "ai.anthropic.allow".
        let toml_str = r#"
[settings]
ai.anthropic.allow = { value = true, modified = "2026-01-01T00:00:00Z" }
"#;
        let result: Result<SettingsFile, _> = toml::from_str(toml_str);
        // This should fail because the nested table structure does not match
        // HashMap<String, SettingEntry>.
        assert!(result.is_err(), "unquoted dotted keys should fail (creates nested tables)");
    }

    #[test]
    fn parse_toml_guest_env_keys() {
        let toml_str = r#"
[settings]
"guest.env.EDITOR" = { value = "vim", modified = "2026-01-01T00:00:00Z" }
"guest.env.TERM" = { value = "xterm-256color", modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse guest env");
        assert_eq!(file.settings.len(), 2);
        assert_eq!(
            file.settings["guest.env.EDITOR"].value,
            SettingValue::Text("vim".into()),
        );
    }

    #[test]
    fn parse_toml_api_key_with_special_chars() {
        // API keys often have dashes, underscores, and mixed case
        let toml_str = r#"
[settings]
"ai.anthropic.api_key" = { value = "sk-ant-api03-ABCD_1234-efgh-5678", modified = "2026-01-01T00:00:00Z" }
"#;
        let file: SettingsFile = toml::from_str(toml_str).expect("should parse API key with special chars");
        assert_eq!(
            file.settings["ai.anthropic.api_key"].value,
            SettingValue::Text("sk-ant-api03-ABCD_1234-efgh-5678".into()),
        );
    }

    #[test]
    fn parse_toml_resolves_with_api_key_type() {
        // Parse from raw TOML, then resolve -- api_key settings must have
        // setting_type == ApiKey, not Text.
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.anthropic.api_key").unwrap();
        assert_eq!(s.setting_type, SettingType::ApiKey, "api_key settings must have ApiKey type");
        assert_eq!(s.effective_value, SettingValue::Text("sk-test".into()));
    }

    #[test]
    fn parse_toml_serialized_format_roundtrips() {
        // Verify that toml::to_string_pretty output parses back correctly
        let file = file_with(vec![
            ("ai.google.api_key", SettingValue::Text("AIzaTest".into())),
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.max_body_capture", SettingValue::Number(4096)),
        ]);
        let serialized = toml::to_string_pretty(&file).unwrap();
        let parsed: SettingsFile = toml::from_str(&serialized)
            .unwrap_or_else(|e| panic!("failed to re-parse serialized TOML:\n{serialized}\nerror: {e}"));
        assert_eq!(file.settings.len(), parsed.settings.len());
        for (key, entry) in &file.settings {
            assert_eq!(&entry.value, &parsed.settings[key].value, "mismatch for {key}");
        }
    }

    #[test]
    fn json_metadata_fields_present_when_empty() {
        // SettingMetadata uses skip_serializing_if = "Vec::is_empty" etc.
        // If empty fields are omitted from JSON, the JS frontend will crash
        // because it accesses metadata.choices.length (undefined.length -> TypeError).
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let json = serde_json::to_string(&resolved).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

        // Find a setting with empty metadata (e.g., api_key settings)
        let api_key = parsed.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
            .unwrap();
        let meta = &api_key["metadata"];

        // These fields MUST be present in JSON (even when empty) or the
        // frontend will crash with undefined.length errors.
        assert!(
            meta.get("choices").is_some(),
            "metadata.choices must be present in JSON (got: {meta})"
        );
        assert!(
            meta.get("domains").is_some(),
            "metadata.domains must be present in JSON (got: {meta})"
        );
    }

    #[test]
    fn resolved_settings_json_serialization() {
        // Tauri sends settings as JSON to the frontend. Verify the full
        // pipeline: parse TOML -> resolve -> serialize to JSON -> has setting_type.
        let toml_str = r#"
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-01T00:00:00Z" }
"ai.anthropic.api_key" = { value = "sk-test", modified = "2026-01-01T00:00:00Z" }
"#;
        let user: SettingsFile = toml::from_str(toml_str).unwrap();
        let resolved = resolve_settings(&user, &empty_file());
        let json = serde_json::to_string(&resolved).expect("should serialize to JSON");

        // Verify key fields are present in the JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();

        // Find the api_key setting
        let api_key = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.api_key")
            .expect("should have ai.anthropic.api_key in JSON");
        assert_eq!(api_key["setting_type"], "apikey", "setting_type must be 'apikey' in JSON");
        assert_eq!(api_key["effective_value"], "sk-test");
        assert_eq!(api_key["enabled"], true);

        // Find a bool setting
        let allow = arr.iter()
            .find(|v| v["id"] == "ai.anthropic.allow")
            .expect("should have ai.anthropic.allow in JSON");
        assert_eq!(allow["setting_type"], "bool");
        assert_eq!(allow["effective_value"], true);

        // Verify all settings have a setting_type field
        for item in arr {
            assert!(
                item.get("setting_type").is_some(),
                "setting {} missing setting_type in JSON",
                item["id"],
            );
        }
    }

    #[test]
    fn load_settings_file_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let file = load_settings_file(&path).unwrap();
        assert!(file.settings.is_empty());
    }

    #[test]
    fn load_settings_file_garbage_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.toml");
        std::fs::write(&path, "not = [valid { toml }").unwrap();
        assert!(load_settings_file(&path).is_err());
    }

    #[test]
    fn load_settings_file_wrong_schema_returns_error() {
        // Valid TOML but wrong structure (settings is a string, not a table)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrong_schema.toml");
        std::fs::write(&path, "settings = \"not a table\"").unwrap();
        assert!(load_settings_file(&path).is_err());
    }

    // -----------------------------------------------------------------------
    // VM settings
    // -----------------------------------------------------------------------

    #[test]
    fn vm_settings_default_scratch_size() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(8));
    }

    #[test]
    fn vm_settings_from_user() {
        let user = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(16))]);
        let resolved = resolve_settings(&user, &empty_file());
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(16));
    }

    #[test]
    fn vm_settings_corp_overrides_user() {
        let user = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(16))]);
        let corp = file_with(vec![("vm.scratch_disk_size_gb", SettingValue::Number(4))]);
        let resolved = resolve_settings(&user, &corp);
        let vs = settings_to_vm_settings(&resolved);
        assert_eq!(vs.scratch_disk_size_gb, Some(4));
    }

    // -----------------------------------------------------------------------
    // J: Domain settings (4)
    // -----------------------------------------------------------------------

    #[test]
    fn domains_setting_drives_allow_list() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("*.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_drives_block_list() {
        // ai.anthropic.allow defaults to false, so domains go to block list
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn domains_setting_parsed_correctly() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("api.anthropic.com , console.anthropic.com , *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("console.anthropic.com");
        assert_eq!(action, Action::Allow);
        let (action, _) = dp.evaluate("new.anthropic.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn domains_setting_empty_skipped() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Empty domains text means nothing added to allow list
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "empty domains should not allow anything");
    }

    // -----------------------------------------------------------------------
    // K: Corp block enforcement (3)
    // -----------------------------------------------------------------------

    #[test]
    fn corp_blocked_domains_always_in_block_list() {
        // Corp locks ai.anthropic.allow = false
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User tries to empty the domains
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains (*.anthropic.com) should still be blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domains must stay blocked");
    }

    #[test]
    fn corp_blocked_domain_not_allowed_via_other_service() {
        // Corp blocks anthropic
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        // User adds api.anthropic.com to google domains and enables google
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.domains", SettingValue::Text("*.googleapis.com,api.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // api.anthropic.com should be blocked even though it's in google domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked domain must not be allowed via other service");
        // google domains should still work
        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn user_disabled_service_domains_in_block_list() {
        // User (not corp) disables a service
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny);
    }

    // -----------------------------------------------------------------------
    // K2: Stress tests -- block > allow > default invariants
    // -----------------------------------------------------------------------

    #[test]
    fn stress_disabled_provider_always_blocked_regardless_of_default() {
        // Provider off + default_action=allow => domains must still be blocked.
        let user = file_with(vec![
            ("network.default_action", SettingValue::Text("allow".into())),
            // anthropic defaults to off
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "disabled provider must be blocked even with default=allow");
    }

    #[test]
    fn stress_enabled_provider_always_allowed_regardless_of_default() {
        // Provider on + default_action=deny => domains must still be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Allow, "enabled provider must be allowed even with default=deny");
    }

    #[test]
    fn stress_corp_block_beats_user_allow() {
        // Corp blocks anthropic, user enables it -- block must win.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(true))]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow");
    }

    #[test]
    fn stress_corp_block_beats_user_allow_with_default_allow() {
        // Corp blocks, user enables, default=allow -- still blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must beat user allow + default allow");
    }

    #[test]
    fn stress_corp_block_via_other_provider_wildcard() {
        // Corp blocks *.anthropic.com via anthropic toggle.
        // User adds *.anthropic.com to openai domains and enables openai.
        // Corp-blocked wildcard must still deny.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.openai.domains", SettingValue::Text("*.openai.com, *.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // anthropic subdomain must be blocked despite being in openai domains
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked wildcard must not be allowed via other provider");
        // openai subdomain should be allowed (not corp-blocked)
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow);
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_emptying_domains() {
        // Corp blocks anthropic. User empties the domains field to try
        // removing the domains from the block list.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default domains should still be blocked (union of default + effective)
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp block must survive user emptying domains");
    }

    #[test]
    fn stress_corp_block_cannot_be_circumvented_by_changing_domains() {
        // Corp blocks anthropic. User changes domains to something else.
        // Both old defaults AND new effective domains must be blocked.
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.anthropic.domains", SettingValue::Text("custom.anthropic.com".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        // Default wildcard still blocked
        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "default domains must remain blocked");
        // User's custom domain also blocked (corp said no anthropic)
        let (action, _) = dp.evaluate("custom.anthropic.com");
        assert_eq!(action, Action::Deny, "user-added domains must also be blocked when corp says no");
    }

    #[test]
    fn stress_user_disable_blocks_even_with_default_allow() {
        // User disables a provider. Even with default_action=allow,
        // that provider's domains must be explicitly blocked.
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Deny, "user-disabled provider must be blocked even with default=allow");
    }

    #[test]
    fn stress_registry_disable_blocks_all_domains() {
        // Disabling a registry blocks ALL its domains, not just some.
        let user = file_with(vec![("registry.github.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("api.github.com");
        assert_eq!(action, Action::Deny);
        let (action, _) = dp.evaluate("raw.githubusercontent.com");
        assert_eq!(action, Action::Deny);
    }

    #[test]
    fn stress_all_providers_disabled_all_blocked() {
        // Disable every provider and registry. All their domains must be blocked.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
            ("registry.github.allow", SettingValue::Bool(false)),
            ("registry.pypi.allow", SettingValue::Bool(false)),
            ("registry.npm.allow", SettingValue::Bool(false)),
            ("registry.crates.allow", SettingValue::Bool(false)),
            ("registry.debian.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // Every known domain should be denied
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org", "registry.npmjs.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when all services disabled");
        }
    }

    #[test]
    fn stress_all_providers_enabled_all_allowed() {
        // Enable every provider. All their domains must be allowed.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("registry.github.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
            "github.com", "api.github.com",
            "pypi.org",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Allow, "{domain} must be allowed when all services enabled");
        }
    }

    #[test]
    fn stress_unknown_domain_follows_default_deny() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        // default_action defaults to "deny"
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Deny, "unknown domain must follow default deny");
    }

    #[test]
    fn stress_unknown_domain_follows_default_allow() {
        let user = file_with(vec![
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let dp = settings_to_domain_policy(&resolved);
        let (action, _) = dp.evaluate("totally-unknown.example.org");
        assert_eq!(action, Action::Allow, "unknown domain must follow default allow");
    }

    #[test]
    fn stress_corp_block_all_providers_user_enables_all() {
        // Corp blocks every AI provider. User enables them all.
        // Corp must win for all.
        let corp = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(false)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("network.default_action", SettingValue::Text("allow".into())),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);
        for domain in &[
            "api.anthropic.com", "api.openai.com",
            "generativelanguage.googleapis.com",
        ] {
            let (action, _) = dp.evaluate(domain);
            assert_eq!(action, Action::Deny, "{domain} must be blocked when corp blocks all providers");
        }
    }

    #[test]
    fn stress_mixed_corp_and_user_decisions() {
        // Corp blocks anthropic only. User enables openai, disables google.
        // anthropic: corp-blocked (deny)
        // openai: user-enabled (allow)
        // google: user-disabled (deny)
        let corp = file_with(vec![("ai.anthropic.allow", SettingValue::Bool(false))]);
        let user = file_with(vec![
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.google.allow", SettingValue::Bool(false)),
        ]);
        let resolved = resolve_settings(&user, &corp);
        let dp = settings_to_domain_policy(&resolved);

        let (action, _) = dp.evaluate("api.anthropic.com");
        assert_eq!(action, Action::Deny, "corp-blocked anthropic must be denied");

        let (action, _) = dp.evaluate("api.openai.com");
        assert_eq!(action, Action::Allow, "user-enabled openai must be allowed");

        let (action, _) = dp.evaluate("generativelanguage.googleapis.com");
        assert_eq!(action, Action::Deny, "user-disabled google must be denied");
    }

    // -----------------------------------------------------------------------
    // L: API key injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_injected_when_toggle_on() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_injected_even_when_toggle_off() {
        // API keys are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![
            // ai.anthropic.allow defaults to false
            ("ai.anthropic.api_key", SettingValue::Text("sk-test-123".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    }

    #[test]
    fn api_key_not_injected_when_empty() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().map_or(false, |e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn google_api_key_sets_gemini_env_var() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.api_key", SettingValue::Text("AIza-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-test");
        // Only GEMINI_API_KEY is set (not GOOGLE_API_KEY) to avoid
        // gemini CLI warning: "Both GOOGLE_API_KEY and GEMINI_API_KEY are set"
        assert!(env.get("GOOGLE_API_KEY").is_none());
    }

    #[test]
    fn openai_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            // ai.openai.allow defaults to false
            ("ai.openai.api_key", SettingValue::Text("sk-oai-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai-test");
    }

    #[test]
    fn google_api_key_injected_when_toggle_off() {
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-off".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-off");
    }

    #[test]
    fn all_three_providers_injected() {
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            ("ai.openai.allow", SettingValue::Bool(true)),
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            ("ai.google.allow", SettingValue::Bool(true)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
        // 3 API keys + 7 built-in env vars (TERM, HOME, PATH, LANG, 3x CA)
        // + 3 CAPSEM_*_ALLOWED provider flags
        assert_eq!(env.len(), 13);
    }

    #[test]
    fn all_three_providers_injected_all_toggles_off() {
        // All toggles off but keys set -- all should still be injected.
        let user = file_with(vec![
            // anthropic defaults to off
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            // google: explicitly disable
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn mixed_toggles_all_keys_injected() {
        // One provider on, two off -- all keys should be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("sk-ant".into())),
            // openai defaults to off
            ("ai.openai.api_key", SettingValue::Text("sk-oai".into())),
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-ant");
        assert_eq!(env.get("OPENAI_API_KEY").unwrap(), "sk-oai");
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza");
    }

    #[test]
    fn provider_allowed_env_vars_injected() {
        // CAPSEM_*_ALLOWED env vars reflect the provider allow toggles.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.openai.allow", SettingValue::Bool(false)),
            ("ai.google.allow", SettingValue::Bool(true)),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "1");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn provider_allowed_defaults_to_zero() {
        // Default allow values: anthropic=false, openai=false, google=true.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("CAPSEM_ANTHROPIC_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_OPENAI_ALLOWED").unwrap(), "0");
        assert_eq!(env.get("CAPSEM_GOOGLE_ALLOWED").unwrap(), "1");
    }

    #[test]
    fn empty_keys_skipped_regardless_of_toggle() {
        // Toggle on but key empty -- should NOT be injected.
        // Toggle off and key empty -- should NOT be injected.
        let user = file_with(vec![
            ("ai.anthropic.allow", SettingValue::Bool(true)),
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
            ("ai.openai.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // Only dynamic env vars from defaults might exist, but no API keys.
        let has_ant = gc.env.as_ref().map_or(false, |e| e.contains_key("ANTHROPIC_API_KEY"));
        let has_oai = gc.env.as_ref().map_or(false, |e| e.contains_key("OPENAI_API_KEY"));
        assert!(!has_ant, "empty anthropic key should not be injected");
        assert!(!has_oai, "empty openai key should not be injected");
    }

    // -----------------------------------------------------------------------
    // M: Gemini CLI boot files
    // -----------------------------------------------------------------------

    #[test]
    fn gemini_boot_files_injected_when_google_enabled() {
        // Google AI is enabled by default, so gemini files should be injected
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_injected_even_when_google_disabled() {
        // Boot files are always injected so user can enable the provider at
        // runtime without rebooting the VM.
        let user = file_with(vec![("ai.google.allow", SettingValue::Bool(false))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_settings_json_user_override() {
        let custom = r#"{"homeDirectoryWarningDismissed":true,"mcpServers":{"myserver":{}}}"#;
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::Text(custom.into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"));
    }

    #[test]
    fn gemini_boot_files_have_correct_paths() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    #[test]
    fn gemini_boot_files_user_override_with_toggle_off() {
        // Custom file content should be injected even when google is disabled.
        let custom = r#"{"mcpServers":{"custom":{}}}"#;
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.gemini.settings_json", SettingValue::Text(custom.into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let gemini_settings = files.iter().find(|f| f.path == "/root/.gemini/settings.json").unwrap();
        assert!(gemini_settings.content.contains("mcpServers"), "custom content should be present");
    }

    #[test]
    fn gemini_boot_files_empty_value_skipped() {
        // If a file setting is explicitly set to empty, it should not be injected.
        let user = file_with(vec![
            ("ai.google.gemini.settings_json", SettingValue::Text("".into())),
            ("ai.google.gemini.projects_json", SettingValue::Text("".into())),
            ("ai.google.gemini.trusted_folders_json", SettingValue::Text("".into())),
            ("ai.google.gemini.installation_id", SettingValue::Text("".into())),
            ("ai.anthropic.claude.settings_json", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // Only TLS CA bundle boot files (if any) should remain.
        let file_paths: Vec<&str> = gc.files.as_ref().map_or(vec![], |f| f.iter().map(|x| x.path.as_str()).collect());
        assert!(!file_paths.contains(&"/root/.gemini/settings.json"));
        assert!(!file_paths.contains(&"/root/.claude/settings.json"));
    }

    #[test]
    fn gemini_boot_files_have_correct_mode() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        for f in &files {
            assert_eq!(f.mode, 0o644, "boot file {} should have mode 0644", f.path);
        }
    }

    #[test]
    fn api_keys_and_boot_files_both_injected_toggle_off() {
        // End-to-end: toggle off, but key + files should all be present.
        let user = file_with(vec![
            ("ai.google.allow", SettingValue::Bool(false)),
            ("ai.google.api_key", SettingValue::Text("AIza-key".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        // API key should be injected
        let env = gc.env.unwrap();
        assert_eq!(env.get("GEMINI_API_KEY").unwrap(), "AIza-key");
        // Boot files (from defaults) should also be injected
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    // -----------------------------------------------------------------------
    // N: File setting type
    // -----------------------------------------------------------------------

    #[test]
    fn file_type_exists_in_setting_type_enum() {
        // The File variant should serialize to "file".
        let st = SettingType::File;
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(json, r#""file""#);
    }

    #[test]
    fn gemini_json_settings_use_file_type() {
        // All .json Gemini settings should be SettingType::File, not Text.
        let defs = setting_definitions();
        for id in &[
            "ai.google.gemini.settings_json",
            "ai.google.gemini.projects_json",
            "ai.google.gemini.trusted_folders_json",
        ] {
            let def = defs.iter().find(|d| d.id == *id).unwrap();
            assert_eq!(
                def.setting_type,
                SettingType::File,
                "{id} should be File type"
            );
        }
    }

    #[test]
    fn gemini_installation_id_stays_text() {
        // installation_id is plain text, not JSON -- stays Text.
        let defs = setting_definitions();
        let def = defs.iter().find(|d| d.id == "ai.google.gemini.installation_id").unwrap();
        assert_eq!(def.setting_type, SettingType::Text);
    }

    #[test]
    fn file_settings_have_guest_path_metadata() {
        // Every File-type setting must declare its guest_path in metadata.
        let defs = setting_definitions();
        for def in &defs {
            if def.setting_type == SettingType::File {
                assert!(
                    def.metadata.guest_path.is_some(),
                    "File setting {} must have guest_path metadata",
                    def.id,
                );
                let path = def.metadata.guest_path.as_ref().unwrap();
                assert!(path.starts_with('/'), "guest_path must be absolute: {path}");
            }
        }
    }

    #[test]
    fn guest_config_collects_file_type_settings() {
        // settings_to_guest_config should pick up File-type settings via
        // metadata.guest_path instead of the hardcoded FILE_MAP.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let files = gc.files.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        // JSON files come from File-type settings
        assert!(paths.contains(&"/root/.gemini/settings.json"));
        assert!(paths.contains(&"/root/.gemini/projects.json"));
        assert!(paths.contains(&"/root/.gemini/trustedFolders.json"));
        // installation_id comes from the legacy FILE_MAP (Text type)
        assert!(paths.contains(&"/root/.gemini/installation_id"));
    }

    // -----------------------------------------------------------------------
    // O: Setting value validation
    // -----------------------------------------------------------------------

    #[test]
    fn validate_file_setting_rejects_invalid_json() {
        // File settings whose guest_path ends in .json must contain valid JSON.
        let err = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::Text("{not valid json".into()),
        );
        assert!(err.is_err(), "invalid JSON should be rejected");
        assert!(err.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn validate_file_setting_accepts_valid_json() {
        let result = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::Text(r#"{"key":"value"}"#.into()),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_file_setting_accepts_empty() {
        // Empty is fine -- means "use default" or "don't inject".
        let result = validate_setting_value(
            "ai.google.gemini.settings_json",
            &SettingValue::Text("".into()),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_json_file_accepts_anything() {
        // installation_id is a Text, not File -- no JSON validation.
        let result = validate_setting_value(
            "ai.google.gemini.installation_id",
            &SettingValue::Text("not json at all".into()),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn validate_non_file_settings_pass_through() {
        // Bool, Number, etc. settings always pass validation.
        let result = validate_setting_value(
            "ai.anthropic.allow",
            &SettingValue::Bool(true),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn file_type_resolved_setting_has_guest_path() {
        // The resolved setting for a File type should carry guest_path in metadata.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let s = resolved.iter().find(|s| s.id == "ai.google.gemini.settings_json").unwrap();
        assert_eq!(s.setting_type, SettingType::File);
        assert_eq!(
            s.metadata.guest_path.as_deref(),
            Some("/root/.gemini/settings.json"),
        );
    }

    // -----------------------------------------------------------------------
    // P: Metadata-driven env var injection
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_settings_have_env_vars_metadata() {
        // API key settings must declare their env var name in metadata.env_vars
        // instead of relying on a hardcoded API_KEY_MAP.
        let defs = setting_definitions();
        let cases = [
            ("ai.anthropic.api_key", "ANTHROPIC_API_KEY"),
            ("ai.openai.api_key", "OPENAI_API_KEY"),
            ("ai.google.api_key", "GEMINI_API_KEY"),
        ];
        for (id, expected_var) in &cases {
            let def = defs.iter().find(|d| d.id == *id)
                .unwrap_or_else(|| panic!("missing setting {id}"));
            assert!(
                def.metadata.env_vars.contains(&expected_var.to_string()),
                "{id} should have env_vars containing {expected_var}, got {:?}",
                def.metadata.env_vars,
            );
        }
    }

    #[test]
    fn builtin_env_settings_exist() {
        // Built-in guest env vars (TERM, HOME, PATH, LANG) must be registered
        // settings, not hardcoded in build_boot_config.
        let defs = setting_definitions();
        let required = ["TERM", "HOME", "PATH", "LANG"];
        for var in &required {
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
            assert!(found, "no setting definition injects env var {var}");
        }
    }

    #[test]
    fn ca_bundle_setting_injects_three_env_vars() {
        // A single CA bundle setting should inject REQUESTS_CA_BUNDLE,
        // NODE_EXTRA_CA_CERTS, and SSL_CERT_FILE.
        let defs = setting_definitions();
        let ca_vars = ["REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE"];
        for var in &ca_vars {
            let found = defs.iter().any(|d| d.metadata.env_vars.contains(&var.to_string()));
            assert!(found, "no setting definition injects env var {var}");
        }
    }

    #[test]
    fn guest_config_env_from_metadata_env_vars() {
        // settings_to_guest_config should inject env vars based on
        // metadata.env_vars, not hardcoded API_KEY_MAP.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("sk-test".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test");
    }

    #[test]
    fn builtin_env_defaults_in_guest_config() {
        // With no user/corp overrides, the built-in env vars should have
        // their default values from the setting definitions.
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
        assert_eq!(env.get("HOME").unwrap(), "/root");
        assert!(env.get("PATH").unwrap().contains("/usr/bin"));
        assert_eq!(env.get("LANG").unwrap(), "C");
    }

    #[test]
    fn ca_bundle_injected_as_three_env_vars() {
        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        let ca_path = "/etc/ssl/certs/ca-certificates.crt";
        assert_eq!(env.get("REQUESTS_CA_BUNDLE").unwrap(), ca_path);
        assert_eq!(env.get("NODE_EXTRA_CA_CERTS").unwrap(), ca_path);
        assert_eq!(env.get("SSL_CERT_FILE").unwrap(), ca_path);
    }

    #[test]
    fn corp_can_override_builtin_env() {
        // Corp should be able to lock down built-in env settings.
        let defs = setting_definitions();
        let term_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"TERM".to_string())).unwrap();
        let corp = file_with(vec![(term_def.id, SettingValue::Text("dumb".into()))]);
        let resolved = resolve_settings(&empty_file(), &corp);
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("TERM").unwrap(), "dumb");
    }

    #[test]
    fn user_can_override_builtin_env() {
        let defs = setting_definitions();
        let path_def = defs.iter().find(|d| d.metadata.env_vars.contains(&"PATH".to_string())).unwrap();
        let user = file_with(vec![(path_def.id, SettingValue::Text("/custom/bin".into()))]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("PATH").unwrap(), "/custom/bin");
    }

    #[test]
    fn empty_env_var_setting_not_injected() {
        // A setting with env_vars metadata but empty value should not be injected.
        let user = file_with(vec![
            ("ai.anthropic.api_key", SettingValue::Text("".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().map_or(false, |e| e.contains_key("ANTHROPIC_API_KEY"));
        assert!(!has_key, "empty API key should not be injected");
    }

    #[test]
    fn dynamic_guest_env_still_works() {
        // Dynamic guest.env.* settings should still be injected alongside
        // metadata-driven env vars.
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        // Built-in env vars should also be present.
        assert!(env.contains_key("TERM"));
    }

    #[test]
    fn each_boot_message_fits_in_frame() {
        // Each individual boot message (SetEnv, FileWrite) must fit in
        // MAX_FRAME_SIZE. The old single-BootConfig frame limit is gone.
        use capsem_proto::{encode_host_msg, MAX_FRAME_SIZE};

        let resolved = resolve_settings(&empty_file(), &empty_file());
        let gc = settings_to_guest_config(&resolved);

        // Each env var as a SetEnv message
        for (key, value) in gc.env.unwrap_or_default() {
            let msg = capsem_proto::HostToGuest::SetEnv { key: key.clone(), value: value.clone() };
            let frame = encode_host_msg(&msg).unwrap();
            assert!(
                frame.len() - 4 <= MAX_FRAME_SIZE as usize,
                "SetEnv({key}) too large: {} bytes",
                frame.len() - 4,
            );
        }

        // Each file as a FileWrite message
        for f in gc.files.unwrap_or_default() {
            let msg = capsem_proto::HostToGuest::FileWrite {
                path: f.path.clone(),
                data: f.content.into_bytes(),
                mode: f.mode,
            };
            let frame = encode_host_msg(&msg).unwrap();
            assert!(
                frame.len() - 4 <= MAX_FRAME_SIZE as usize,
                "FileWrite({}) too large: {} bytes",
                f.path,
                frame.len() - 4,
            );
        }
    }

    #[test]
    fn all_env_vars_metadata_refers_to_text_settings() {
        // Every setting with env_vars metadata must have a text-like type
        // (Text, ApiKey, Password, Url, Email).
        let defs = setting_definitions();
        for def in &defs {
            if !def.metadata.env_vars.is_empty() {
                assert!(
                    matches!(def.setting_type, SettingType::Text | SettingType::ApiKey | SettingType::Password | SettingType::Url | SettingType::Email),
                    "setting {} has env_vars but type {:?} (should be text-like)",
                    def.id, def.setting_type,
                );
            }
        }
    }

    // -------------------------------------------------------------------
    // Boot handshake validation in settings layer
    // -------------------------------------------------------------------

    #[test]
    fn settings_rejects_blocked_env_var() {
        // guest.env.LD_PRELOAD in user.toml should be silently dropped.
        let user = file_with(vec![
            ("guest.env.LD_PRELOAD", SettingValue::Text("/evil/lib.so".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().map_or(false, |e| e.contains_key("LD_PRELOAD"));
        assert!(!has_key, "LD_PRELOAD should be dropped by validation");
    }

    #[test]
    fn settings_rejects_ld_library_path() {
        let user = file_with(vec![
            ("guest.env.LD_LIBRARY_PATH", SettingValue::Text("/evil".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let has_key = gc.env.as_ref().map_or(false, |e| e.contains_key("LD_LIBRARY_PATH"));
        assert!(!has_key, "LD_LIBRARY_PATH should be dropped by validation");
    }

    #[test]
    fn settings_accepts_normal_dynamic_env() {
        let user = file_with(vec![
            ("guest.env.EDITOR", SettingValue::Text("vim".into())),
        ]);
        let resolved = resolve_settings(&user, &empty_file());
        let gc = settings_to_guest_config(&resolved);
        let env = gc.env.unwrap();
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
    }
}
