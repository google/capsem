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
    let fields: std::collections::BTreeMap<String, Option<serde::de::IgnoredAny>> =
        rmp_serde::from_slice(&frame[4..]).unwrap();
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

#[test]
fn repeated_events_store_only_mutation_count_and_time_range() {
    use crate::repeated::EventRun;

    let mut run = EventRun::new(100, "dns.query".to_string());
    let encoded = run.encode().unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert!(fields.get("count").is_none());
    assert!(fields.get("last_timestamp_unix_ms").is_none());
    assert_eq!(run.last_timestamp_unix_ms(), 100);
    assert!(run.validate());

    assert!(!run.absorb(101, &"other".to_string()));
    assert!(!run.absorb(99, &"dns.query".to_string()));
    assert!(run.absorb(101, &"dns.query".to_string()));
    assert!(run.absorb(103, &"dns.query".to_string()));
    let encoded = run.encode().unwrap();
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(fields["count"], 3);
    assert_eq!(fields["first_timestamp_unix_ms"], 100);
    assert_eq!(fields["last_timestamp_unix_ms"], 103);
    assert_eq!(EventRun::<String>::decode(&encoded).unwrap(), run);

    let zero = rmp_serde::to_vec_named(&serde_json::json!({
        "first_timestamp_unix_ms": 1,
        "count": 0,
        "event": "dns.query"
    }))
    .unwrap();
    assert!(EventRun::<String>::decode(&zero).is_err());
    let reversed = rmp_serde::to_vec_named(&serde_json::json!({
        "first_timestamp_unix_ms": 2,
        "last_timestamp_unix_ms": 1,
        "count": 2,
        "event": "dns.query"
    }))
    .unwrap();
    assert!(EventRun::<String>::decode(&reversed).is_err());
    assert!(EventRun::<String>::decode(&vec![0; crate::repeated::MAX_ENCODED_EVENT_BYTES + 1]).is_err());
}

#[test]
fn forensic_msgpack_omits_defaults_but_preserves_mutations_and_opaque_json() {
    use crate::forensic::SecurityForensicEvent;

    let json = serde_json::json!({
        "event_type": "mcp.request",
        "credential_ref": null,
        "credential_observations": [],
        "action_trace": [],
        "decision": {"kind": "allow", "reason": null},
        "http": null,
        "mcp": {
            "method": "tools/call",
            "server_name": null,
            "request": {"arguments": {"nullable": null, "empty": []}},
            "response": null,
            "event": {"valid": true},
            "tool_call": {"valid": false}
        },
        "future_extension": {"meaningful": 0, "explicit_null": null}
    });
    let event = SecurityForensicEvent::from_json(&json.to_string(), "mcp.request").unwrap();
    let encoded = event.encode().unwrap();
    assert!(
        encoded.len() < json.to_string().len(),
        "omission must save stored bytes"
    );
    let fields: serde_json::Value = rmp_serde::from_slice(&encoded).unwrap();
    assert!(fields.get("credential_ref").is_none());
    assert!(fields.get("credential_observations").is_none());
    assert!(fields.get("http").is_none());
    assert!(fields["decision"].get("reason").is_none());
    assert!(fields["mcp"].get("server_name").is_none());
    assert!(fields["mcp"].get("tool_call").is_none());
    assert_eq!(
        fields["mcp"]["request"]["arguments"]["nullable"],
        serde_json::Value::Null
    );
    assert_eq!(fields["mcp"]["request"]["arguments"]["empty"], serde_json::json!([]));
    assert_eq!(fields["future_extension"]["meaningful"], 0);
    assert_eq!(fields["future_extension"]["explicit_null"], serde_json::Value::Null);
    let decoded = SecurityForensicEvent::decode(&encoded).unwrap();
    assert_eq!(decoded.event_type, "mcp.request");
    assert!(decoded.credential_observations.is_empty());
    assert_eq!(decoded.to_json().unwrap(), event.to_json().unwrap());

    let fragment = SecurityForensicEvent::from_json(r#"{"http":{"host":"api.openai.com"}}"#, "http.request").unwrap();
    assert_eq!(fragment.event_type, "http.request");
    assert!(fragment.decision.is_none());
    assert_eq!(
        SecurityForensicEvent::decode(&fragment.encode().unwrap()).unwrap(),
        fragment
    );
}
