use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::types::*;
use super::loader::load_settings_files;
use super::resolver::resolve_settings;

/// A single config validation issue.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ConfigIssue {
    /// Setting ID (e.g. "ai.anthropic.api_key").
    pub id: String,
    /// "error" | "warning".
    pub severity: String,
    /// Human-readable message shown in the UI.
    pub message: String,
    /// Documentation URL for getting an API key (shown as "Get key" link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
}

/// Validate all resolved settings and return a list of issues.
///
/// Checks: number ranges, choice validity, JSON file content, API key format,
/// enabled-provider-with-empty-key, nul bytes in text.
pub fn config_lint(resolved: &[ResolvedSetting]) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();

    // Build a lookup for toggle values (for enabled-provider checks).
    let toggle_values: HashMap<String, bool> = resolved
        .iter()
        .filter(|s| s.setting_type == SettingType::Bool)
        .filter_map(|s| s.effective_value.as_bool().map(|b| (s.id.clone(), b)))
        .collect();

    for s in resolved {
        let text_value = match &s.effective_value {
            SettingValue::Text(t) => Some(t.as_str()),
            _ => None,
        };

        // -- Nul byte check (all text values) --
        if let Some(text) = text_value {
            if text.contains('\0') {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: value contains invalid characters", s.id),
                    docs_url: None,
                });
            }
        }

        // -- Number range --
        if s.setting_type == SettingType::Number {
            if let Some(n) = s.effective_value.as_number() {
                if let Some(min) = s.metadata.min {
                    if n < min {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!(
                                "{}: value {} is below minimum {}",
                                s.id, n, min
                            ),
                            docs_url: None,
                        });
                    }
                }
                if let Some(max) = s.metadata.max {
                    if n > max {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!(
                                "{}: value {} exceeds maximum {}",
                                s.id, n, max
                            ),
                            docs_url: None,
                        });
                    }
                }
            }
        }

        // -- Choice validation --
        if !s.metadata.choices.is_empty() {
            if let Some(text) = text_value {
                if !s.metadata.choices.iter().any(|c| c == text) {
                    issues.push(ConfigIssue {
                        id: s.id.clone(),
                        severity: "error".into(),
                        message: format!(
                            "{}: '{}' is not a valid choice ({})",
                            s.id,
                            text,
                            s.metadata.choices.join(", ")
                        ),
                        docs_url: None,
                    });
                }
            }
        }

        // -- File value validation (path + JSON content) --
        if let SettingValue::File { path: file_path, content: file_content } = &s.effective_value {
            // Path validation
            if !file_path.starts_with('/') {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: file path must be absolute", s.id),
                    docs_url: None,
                });
            }
            if file_path.contains("..") {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "error".into(),
                    message: format!("{}: file path must not contain '..'", s.id),
                    docs_url: None,
                });
            }
            if !file_path.starts_with("/root/") && !file_path.starts_with("/root/.") && !file_path.starts_with("/etc/") {
                issues.push(ConfigIssue {
                    id: s.id.clone(),
                    severity: "warning".into(),
                    message: format!("{}: unusual file path (expected under /root/ or /etc/)", s.id),
                    docs_url: None,
                });
            }
            // JSON content validation for .json paths
            if file_path.ends_with(".json") && !file_content.is_empty() {
                match serde_json::from_str::<serde_json::Value>(file_content) {
                    Ok(val) => {
                        if !val.is_object() && !val.is_array() {
                            issues.push(ConfigIssue {
                                id: s.id.clone(),
                                severity: "warning".into(),
                                message: format!(
                                    "{}: JSON parsed but is not an object",
                                    s.id
                                ),
                                docs_url: None,
                            });
                        }
                    }
                    Err(e) => {
                        issues.push(ConfigIssue {
                            id: s.id.clone(),
                            severity: "error".into(),
                            message: format!("{}: invalid JSON -- {}", s.id, e),
                            docs_url: None,
                        });
                    }
                }
            }
        }

        // -- API key whitespace check --
        if s.setting_type == SettingType::ApiKey {
            if let Some(text) = text_value {
                if !text.is_empty()
                    && (text.contains(' ')
                        || text.contains('\n')
                        || text.contains('\r')
                        || text.contains('\t'))
                {
                    issues.push(ConfigIssue {
                        id: s.id.clone(),
                        severity: "warning".into(),
                        message: format!(
                            "{}: key contains whitespace -- check for copy-paste errors",
                            s.id
                        ),
                        docs_url: None,
                    });
                }
            }
        }

        // -- Enabled provider with empty API key --
        if s.setting_type == SettingType::ApiKey {
            if let Some(text) = text_value {
                if text.trim().is_empty() {
                    // Check if the parent toggle is on
                    if let Some(ref parent_id) = s.enabled_by {
                        if toggle_values.get(parent_id).copied().unwrap_or(false) {
                            issues.push(ConfigIssue {
                                id: s.id.clone(),
                                severity: "warning".into(),
                                message: format!("{} not set", s.name),
                                docs_url: s.metadata.docs_url.clone(),
                            });
                        }
                    }
                }
            }
        }

        // -- URL validation --
        if s.setting_type == SettingType::Url {
            if let Some(text) = text_value {
                if !text.is_empty()
                    && !text.starts_with("http://")
                    && !text.starts_with("https://")
                {
                    issues.push(ConfigIssue {
                        id: s.id.clone(),
                        severity: "warning".into(),
                        message: format!("{}: not a valid URL", s.id),
                        docs_url: None,
                    });
                }
            }
        }
    }

    issues
}

/// Run lint on current merged settings.
pub fn load_merged_lint() -> Vec<ConfigIssue> {
    let (user, corp) = load_settings_files();
    let resolved = resolve_settings(&user, &corp);
    config_lint(&resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_resolved(id: &str, typ: SettingType, value: SettingValue) -> ResolvedSetting {
        ResolvedSetting {
            id: id.to_string(),
            category: "test".into(),
            name: id.to_string(),
            description: "".into(),
            setting_type: typ,
            default_value: value.clone(),
            effective_value: value,
            source: PolicySource::Default,
            modified: None,
            corp_locked: false,
            enabled_by: None,
            enabled: true,
            metadata: SettingMetadata::default(),
            collapsed: false,
            history: vec![],
        }
    }

    #[test]
    fn lint_empty_settings_no_issues() {
        let issues = config_lint(&[]);
        assert!(issues.is_empty());
    }

    #[test]
    fn lint_nul_byte_in_text() {
        let s = make_resolved("test.key", SettingType::Text, SettingValue::Text("hello\0world".into()));
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.severity == "error" && i.message.contains("invalid characters")));
    }

    #[test]
    fn lint_number_below_min() {
        let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(0));
        s.metadata.min = Some(1);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("below minimum")));
    }

    #[test]
    fn lint_number_above_max() {
        let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(100));
        s.metadata.max = Some(50);
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("exceeds maximum")));
    }

    #[test]
    fn lint_number_in_range_no_issue() {
        let mut s = make_resolved("test.num", SettingType::Number, SettingValue::Number(5));
        s.metadata.min = Some(1);
        s.metadata.max = Some(10);
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn lint_invalid_choice() {
        let mut s = make_resolved("test.choice", SettingType::Text, SettingValue::Text("bad".into()));
        s.metadata.choices = vec!["good".into(), "ok".into()];
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("not a valid choice")));
    }

    #[test]
    fn lint_valid_choice_no_issue() {
        let mut s = make_resolved("test.choice", SettingType::Text, SettingValue::Text("good".into()));
        s.metadata.choices = vec!["good".into(), "ok".into()];
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }

    #[test]
    fn lint_file_path_traversal() {
        let s = make_resolved("test.file", SettingType::File, SettingValue::File {
            path: "/root/../etc/shadow".into(),
            content: "".into(),
        });
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("must not contain '..'")));
    }

    #[test]
    fn lint_file_path_not_absolute() {
        let s = make_resolved("test.file", SettingType::File, SettingValue::File {
            path: "relative/path.txt".into(),
            content: "".into(),
        });
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("must be absolute")));
    }

    #[test]
    fn lint_file_invalid_json_content() {
        let s = make_resolved("test.file", SettingType::File, SettingValue::File {
            path: "/root/.config/settings.json".into(),
            content: "not json {{{".into(),
        });
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("invalid JSON")));
    }

    #[test]
    fn lint_api_key_with_whitespace() {
        let s = make_resolved("ai.test.key", SettingType::ApiKey, SettingValue::Text("sk-abc 123\n".into()));
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("whitespace")));
    }

    #[test]
    fn lint_url_not_http() {
        let s = make_resolved("test.url", SettingType::Url, SettingValue::Text("ftp://example.com".into()));
        let issues = config_lint(&[s]);
        assert!(issues.iter().any(|i| i.message.contains("not a valid URL")));
    }

    #[test]
    fn lint_url_valid_https() {
        let s = make_resolved("test.url", SettingType::Url, SettingValue::Text("https://example.com".into()));
        let issues = config_lint(&[s]);
        assert!(issues.is_empty());
    }
}
