use super::*;

#[test]
fn named_msgpack_omits_default_fields_and_restores_them_on_decode() {
    let audit = AuditRecord {
        timestamp_us: 1,
        pid: 2,
        ppid: 1,
        uid: 0,
        exe: "/bin/sh".into(),
        comm: None,
        argv: "sh".into(),
        cwd: None,
        tty: None,
        session_id: None,
        parent_exe: None,
        audit_id: "1:1".into(),
    };
    let frame = encode_audit_record(&audit).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&frame[4..]).unwrap();
    assert!(
        fields.get("comm").is_none(),
        "absent optional fields occupy no wire bytes"
    );
    assert!(fields.get("cwd").is_none());
    assert!(fields.get("tty").is_none());
    assert!(fields.get("session_id").is_none());
    assert!(fields.get("parent_exe").is_none());
    assert_eq!(decode_audit_record(&frame[4..]).unwrap(), audit);

    let req = DnsRequest {
        id: 0,
        raw: vec![1, 2],
        proto: "udp".into(),
        process_name: None,
    };
    let frame = encode_dns_request(&req).unwrap();
    let fields: std::collections::BTreeMap<String, serde::de::IgnoredAny> = rmp_serde::from_slice(&frame[4..]).unwrap();
    assert!(!fields.contains_key("id"));
    assert!(!fields.contains_key("process_name"));
    assert_eq!(decode_dns_request(&frame[4..]).unwrap(), req);
}

#[test]
fn ipc_optional_none_is_absent_and_some_survives() {
    use crate::ipc::ProcessToService;

    let message = ProcessToService::WriteFileResult {
        id: 7,
        success: true,
        error: None,
    };
    let encoded = rmp_serde::to_vec_named(&message).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert!(fields["WriteFileResult"].get("error").is_none());
    assert!(matches!(
        rmp_serde::from_slice::<ProcessToService>(&encoded).unwrap(),
        ProcessToService::WriteFileResult { error: None, .. }
    ));

    let with_error = ProcessToService::WriteFileResult {
        id: 7,
        success: false,
        error: Some("denied".into()),
    };
    let encoded = rmp_serde::to_vec_named(&with_error).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(fields["WriteFileResult"]["error"], "denied");
}

#[test]
fn aggregator_contracts_omit_empty_and_nonzero_defaults() {
    use crate::mcp_contracts::{McpServerDef, ToolAnnotations};

    let server = McpServerDef {
        name: "builtin".into(),
        url: String::new(),
        command: None,
        args: Vec::new(),
        env: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        auth: None,
        enabled: true,
        source: "builtin".into(),
        pool_size: None,
        pool_safe_tools: Vec::new(),
    };
    let encoded = rmp_serde::to_vec_named(&server).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(fields.as_object().unwrap().len(), 3);
    assert!(fields.get("url").is_none());
    assert!(fields.get("args").is_none());
    assert!(fields.get("auth").is_none());
    let decoded: McpServerDef = rmp_serde::from_slice(&encoded).unwrap();
    assert!(decoded.url.is_empty() && decoded.args.is_empty() && decoded.auth.is_none());

    let defaults = ToolAnnotations::default();
    let encoded = rmp_serde::to_vec_named(&defaults).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert!(fields.as_object().unwrap().is_empty());
    assert_eq!(rmp_serde::from_slice::<ToolAnnotations>(&encoded).unwrap(), defaults);

    let changed = ToolAnnotations {
        destructive_hint: false,
        ..defaults
    };
    let encoded = rmp_serde::to_vec_named(&changed).unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(fields["destructive_hint"], false);
    assert_eq!(rmp_serde::from_slice::<ToolAnnotations>(&encoded).unwrap(), changed);
}
