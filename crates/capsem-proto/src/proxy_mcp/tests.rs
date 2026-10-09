use super::*;

#[test]
fn hello_accepts_one_bounded_unique_builtin_set() {
    let hello = ProxyMcpHello::new(vec!["capsem".into(), "files".into()]).unwrap();
    assert_eq!(hello.version, PROXY_MCP_VERSION);
    assert_eq!(hello.builtin_servers, ["capsem", "files"]);
}

#[test]
fn hello_rejects_untrusted_shape() {
    assert_eq!(
        ProxyMcpHello {
            version: PROXY_MCP_VERSION + 1,
            builtin_servers: Vec::new(),
        }
        .validate(),
        Err(ProxyMcpError::Version {
            expected: PROXY_MCP_VERSION,
            actual: PROXY_MCP_VERSION + 1,
        })
    );
    assert_eq!(
        ProxyMcpHello::new(vec!["capsem".into(), "capsem".into()]),
        Err(ProxyMcpError::DuplicateServerName)
    );
    assert_eq!(
        ProxyMcpHello::new(vec![String::new()]),
        Err(ProxyMcpError::InvalidServerName)
    );
    assert_eq!(
        ProxyMcpHello::new(vec!["x".repeat(PROXY_MCP_MAX_SERVER_NAME_BYTES + 1)]),
        Err(ProxyMcpError::InvalidServerName)
    );
    assert_eq!(
        ProxyMcpHello::new(vec!["x".into(); PROXY_MCP_MAX_BUILTIN_SERVERS + 1]),
        Err(ProxyMcpError::TooManyBuiltinServers(PROXY_MCP_MAX_BUILTIN_SERVERS + 1))
    );
}
