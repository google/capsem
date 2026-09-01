use super::*;
use capsem_proto::mcp::{McpAuthConfig, McpAuthKind};

#[test]
fn rejects_secret_bearing_headers_case_insensitively() {
    let config = McpProfileConfig {
        servers: vec![McpManualServer {
            name: "remote".into(),
            url: "https://example.test/mcp".into(),
            headers: [("Authorization".into(), "raw secret".into())].into(),
            auth: None,
            enabled: true,
        }],
        ..McpProfileConfig::default()
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
