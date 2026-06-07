//! Cross-language conformance test for the settings schema.
//!
//! Parses the same golden fixture used by Python and TypeScript tests.
//! Uses local test-only structs matching the new two-node schema
//! (GroupNode + SettingNode), not the live app's 4-variant SettingsNode.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test-only structs (new two-node schema)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
enum TestNode {
    #[serde(rename = "group")]
    Group {
        key: String,
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        enabled_by: Option<String>,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default)]
        collapsed: bool,
        children: Vec<TestNode>,
    },
    #[serde(rename = "setting")]
    Setting(Box<TestSettingNode>),
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TestSettingNode {
    key: String,
    name: String,
    description: String,
    setting_type: String,
    #[serde(default)]
    default_value: Option<serde_json::Value>,
    #[serde(default)]
    effective_value: Option<serde_json::Value>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    modified: Option<String>,
    #[serde(default)]
    corp_locked: bool,
    #[serde(default)]
    enabled_by: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    metadata: TestMetadata,
    #[serde(default)]
    history: Vec<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct TestMetadata {
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    min: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
    #[serde(default)]
    rules: HashMap<String, serde_json::Value>,
    #[serde(default)]
    env_vars: Vec<String>,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    docs_url: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    filetype: Option<String>,
    #[serde(default)]
    widget: Option<String>,
    #[serde(default)]
    side_effect: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    mask: bool,
    #[serde(default)]
    validator: Option<String>,
    // Action-specific
    #[serde(default)]
    action: Option<String>,
    // MCP tool-specific
    #[serde(default)]
    origin: Option<String>,
    // MCP server-specific (legacy)
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    headers: HashMap<String, String>,
}

#[derive(Deserialize)]
struct TestRoot {
    settings: Vec<TestNode>,
}

#[derive(Deserialize)]
struct ExpectedLeaf {
    key: String,
    name: String,
    setting_type: String,
    enabled_by: Option<String>,
}

#[derive(Deserialize)]
struct Expected {
    total_settings: usize,
    by_type: HashMap<String, usize>,
    group_count: usize,
    settings: Vec<ExpectedLeaf>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const GOLDEN: &str = include_str!("../../../tests/settings_spec/golden.json");
const EXPECTED: &str = include_str!("../../../tests/settings_spec/expected.json");

fn parse_golden() -> TestRoot {
    serde_json::from_str(GOLDEN).expect("golden.json should parse")
}

fn parse_expected() -> Expected {
    serde_json::from_str(EXPECTED).expect("expected.json should parse")
}

fn extract_settings(nodes: &[TestNode]) -> Vec<&TestSettingNode> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            TestNode::Setting(s) => out.push(s.as_ref()),
            TestNode::Group { children, .. } => out.extend(extract_settings(children)),
        }
    }
    out
}

fn count_groups(nodes: &[TestNode]) -> usize {
    let mut count = 0;
    for node in nodes {
        if let TestNode::Group { children, .. } = node {
            count += 1;
            count += count_groups(children);
        }
    }
    count
}

fn count_by_type(settings: &[&TestSettingNode]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in settings {
        *counts.entry(s.setting_type.clone()).or_default() += 1;
    }
    counts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn golden_fixture_parses() {
    let root = parse_golden();
    assert!(!root.settings.is_empty());
}

#[test]
fn total_setting_count() {
    let root = parse_golden();
    let expected = parse_expected();
    let settings = extract_settings(&root.settings);
    assert_eq!(settings.len(), expected.total_settings);
}

#[test]
fn per_type_counts() {
    let root = parse_golden();
    let expected = parse_expected();
    let settings = extract_settings(&root.settings);
    let counts = count_by_type(&settings);
    assert_eq!(counts, expected.by_type);
}

#[test]
fn group_count() {
    let root = parse_golden();
    let expected = parse_expected();
    assert_eq!(count_groups(&root.settings), expected.group_count);
}

#[test]
fn setting_fields_match_expected() {
    let root = parse_golden();
    let expected = parse_expected();
    let settings = extract_settings(&root.settings);
    let by_key: HashMap<&str, &&TestSettingNode> =
        settings.iter().map(|s| (s.key.as_str(), s)).collect();

    for exp in &expected.settings {
        let actual = by_key
            .get(exp.key.as_str())
            .unwrap_or_else(|| panic!("missing setting: {}", exp.key));
        assert_eq!(actual.name, exp.name, "name mismatch for {}", exp.key);
        assert_eq!(
            actual.setting_type, exp.setting_type,
            "type mismatch for {}",
            exp.key
        );
        assert_eq!(
            actual.enabled_by, exp.enabled_by,
            "enabled_by mismatch for {}",
            exp.key
        );
    }
}

#[test]
fn all_13_setting_types_present() {
    let expected_types = [
        "text",
        "number",
        "url",
        "email",
        "apikey",
        "bool",
        "file",
        "kv_map",
        "string_list",
        "int_list",
        "float_list",
        "action",
        "mcp_tool",
    ];
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    let present: std::collections::HashSet<&str> =
        settings.iter().map(|s| s.setting_type.as_str()).collect();
    for t in &expected_types {
        assert!(present.contains(t), "missing setting_type: {t}");
    }
}

#[test]
fn action_settings_have_action_kind() {
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    let actions: Vec<_> = settings
        .iter()
        .filter(|s| s.setting_type == "action")
        .collect();
    assert!(!actions.is_empty());
    for a in &actions {
        assert!(
            a.metadata.action.is_some(),
            "action {} missing metadata.action",
            a.key
        );
    }
}

#[test]
fn mcp_tool_settings_have_origin() {
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    let tools: Vec<_> = settings
        .iter()
        .filter(|s| s.setting_type == "mcp_tool")
        .collect();
    assert!(!tools.is_empty());
    for t in &tools {
        assert!(
            t.metadata.origin.is_some(),
            "mcp_tool {} missing metadata.origin",
            t.key
        );
    }
}

#[test]
fn file_setting_has_path_content() {
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    let files: Vec<_> = settings
        .iter()
        .filter(|s| s.setting_type == "file")
        .collect();
    assert!(!files.is_empty());
    for f in &files {
        let dv = f
            .default_value
            .as_ref()
            .expect("file setting should have default_value");
        assert!(dv.get("path").is_some(), "file missing path");
        assert!(dv.get("content").is_some(), "file missing content");
    }
}

#[test]
fn roundtrip_serialize_deserialize() {
    let root = parse_golden();
    let json = serde_json::to_string(&root.settings).unwrap();
    let reparsed: Vec<TestNode> = serde_json::from_str(&json).unwrap();
    let settings1 = extract_settings(&root.settings);
    let settings2 = extract_settings(&reparsed);
    assert_eq!(settings1.len(), settings2.len());
    for (s1, s2) in settings1.iter().zip(settings2.iter()) {
        assert_eq!(s1.key, s2.key);
        assert_eq!(s1.setting_type, s2.setting_type);
    }
}

#[test]
fn hidden_setting_exists() {
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    assert!(
        settings.iter().any(|s| s.metadata.hidden),
        "no hidden setting found"
    );
}

#[test]
fn builtin_setting_exists() {
    let root = parse_golden();
    let settings = extract_settings(&root.settings);
    assert!(
        settings.iter().any(|s| s.metadata.builtin),
        "no builtin setting found"
    );
}
