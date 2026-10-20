use super::*;
use capsem_proto::mcp::{McpAuthConfig, McpAuthKind};

#[test]
fn rejects_secret_bearing_headers_case_insensitively() {
    let config = McpConfig {
        servers: vec![McpManualServer {
            name: "remote".into(),
            url: "https://example.test/mcp".into(),
            headers: [("Authorization".into(), "raw secret".into())].into(),
            auth: None,
            enabled: true,
        }],
        ..McpConfig::default()
    };
    assert!(config.validate("profile").unwrap_err().contains("secret-bearing"));
}

#[test]
fn accepts_brokered_auth_and_rejects_raw_tokens() {
    let mut server = McpManualServer {
        name: "remote".into(),
        url: "https://example.test/mcp".into(),
        headers: Default::default(),
        auth: Some(McpAuthConfig {
            kind: McpAuthKind::Bearer,
            credential_ref: format!("credential:blake3:{}", "a".repeat(64)),
        }),
        enabled: true,
    };
    assert!(server.validate("profile").is_ok());
    server.auth.as_mut().unwrap().credential_ref = "plain-token".into();
    assert!(server.validate("profile").unwrap_err().contains("credential:blake3"));
}

fn server(name: &str, url: &str) -> McpManualServer {
    McpManualServer {
        name: name.into(),
        url: url.into(),
        headers: Default::default(),
        auth: None,
        enabled: true,
    }
}

#[test]
fn merged_is_none_without_either_section() {
    assert_eq!(McpConfig::merged(None, None), None);
}

#[test]
fn merged_keeps_a_lone_section_whole() {
    let user = McpConfig {
        servers: vec![server("wiki", "https://wiki.test/mcp")],
        ..McpConfig::default()
    };
    assert_eq!(McpConfig::merged(Some(&user), None), Some(user.clone()));
    assert_eq!(McpConfig::merged(None, Some(&user)), Some(user));
}

#[test]
fn merged_lets_corp_replace_servers_toggles_and_interval() {
    let user = McpConfig {
        health_check_interval_secs: Some(10),
        servers: vec![
            server("wiki", "https://user.test/mcp"),
            server("notes", "https://notes.test/mcp"),
        ],
        server_enabled: [("local".to_string(), true), ("notes".to_string(), true)].into(),
    };
    let corp = McpConfig {
        health_check_interval_secs: Some(60),
        servers: vec![server("wiki", "https://corp.test/mcp")],
        server_enabled: [("local".to_string(), false)].into(),
    };
    let merged = McpConfig::merged(Some(&user), Some(&corp)).unwrap();
    assert_eq!(merged.health_check_interval_secs, Some(60));
    let urls: Vec<_> = merged
        .servers
        .iter()
        .map(|server| (server.name.as_str(), server.url.as_str()))
        .collect();
    assert_eq!(
        urls,
        vec![("notes", "https://notes.test/mcp"), ("wiki", "https://corp.test/mcp")]
    );
    assert_eq!(merged.server_enabled.get("local"), Some(&false));
    assert_eq!(merged.server_enabled.get("notes"), Some(&true));
}

#[test]
fn merged_keeps_the_user_interval_when_corp_sets_none() {
    let user = McpConfig {
        health_check_interval_secs: Some(10),
        ..McpConfig::default()
    };
    let merged = McpConfig::merged(Some(&user), Some(&McpConfig::default())).unwrap();
    assert_eq!(merged.health_check_interval_secs, Some(10));
}
