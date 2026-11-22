use super::*;
use serde_json::{json, Value};

fn catalog(definition: Value) -> Value {
    json!({"schema_version":1,"kind":"mcp","entries":[{
        "metadata":{"id":"document-review","version":"1","name":"Document review","publisher":"Example",
            "provenance":"Example catalog","license":"Apache-2.0","status":"active",
            "compatibility":{"min_runtime_contract":1}},
        "definition":definition
    }]})
}

fn validate(value: Value) -> Result<(), String> {
    serde_json::from_value::<CatalogDocument<McpCatalogEntry>>(value)
        .map_err(|error| error.to_string())?
        .validate()
        .map_err(str::to_owned)
}

#[test]
fn catalog_mcp_requirements_describe_inputs_without_values_or_credentials() {
    let requirement = json!({"description":"Workspace name","required":true});
    let remote = json!({"kind":"remote","url":"https://example.org/mcp",
        "headers":{"X-Workspace":requirement},"secret_binding":{"description":"Service token","required":true}});
    let sandbox = json!({"kind":"sandbox","executable":"review","args":["--stdio"],"workdir":"/workspace",
        "variables":{"WORKSPACE":requirement},"secret_variables":{"TOKEN":{"description":"Service token","required":false}}});
    validate(catalog(remote.clone())).unwrap();
    validate(catalog(sandbox.clone())).unwrap();

    for definition in [
        json!({"kind":"remote","url":"https://example.org/mcp","binding_id":"local-state"}),
        json!({"kind":"remote","url":"https://user:secret@example.org/mcp"}),
        json!({"kind":"remote","url":"file:///workspace/mcp"}),
        json!({"kind":"remote","url":"https://example.org/mcp","headers":{"Authorization":requirement}}),
        json!({"kind":"remote","url":"https://example.org/mcp","headers":{"X-Foo":requirement,"x-foo":requirement}}),
        json!({"kind":"remote","url":"https://example.org/mcp","headers":{"X-Foo":{"description":"Value","required":true,"value":"unexpected"}}}),
        json!({"kind":"sandbox","executable":"review","workdir":"/a/../b"}),
        json!({"kind":"sandbox","executable":"review","variables":{"TOKEN":requirement},"secret_variables":{"TOKEN":requirement}}),
        json!({"kind":"sandbox","executable":"review","secret_variables":{"INVALID-NAME":requirement}}),
    ] {
        assert!(validate(catalog(definition)).is_err());
    }
    let raw = serde_json::to_string(&catalog(remote)).unwrap();
    let duplicate = raw.replace(
        r#""X-Workspace":"#,
        r#""X-Workspace":{"description":"duplicate","required":true},"X-Workspace":"#,
    );
    assert!(serde_json::from_str::<CatalogDocument<McpCatalogEntry>>(&duplicate).is_err());
}

#[test]
fn catalog_mcp_validates_optional_published_tools_and_revoked_tombstones() {
    let mut document = catalog(json!({"kind":"sandbox","executable":"review","args":["--stdio"]}));
    let reviewed = json!({"name":"review_document","description":"Read and summarize a document"});
    document["entries"][0]["published_tools"] = json!([reviewed]);
    validate(document.clone()).unwrap();
    document["entries"][0]["published_tools"] = json!([reviewed, reviewed]);
    assert!(validate(document.clone()).is_err());
    document["entries"][0]["published_tools"] = json!([{"name":"\ninvalid"}]);
    assert!(validate(document.clone()).is_err());
    document["entries"][0]["published_tools"] = json!((0..129)
        .map(|index| json!({"name":format!("tool_{index}")}))
        .collect::<Vec<_>>());
    assert!(validate(document.clone()).is_err());
    let entry = document["entries"][0].as_object_mut().unwrap();
    entry.remove("definition");
    entry.remove("published_tools");
    assert!(validate(document.clone()).is_err());
    document["entries"][0]["metadata"]["status"] = "revoked".into();
    document["entries"][0]["metadata"]["status_reason"] = "Unavailable".into();
    validate(document.clone()).unwrap();
    document["entries"][0]["metadata"]["id"] = "local".into();
    assert!(validate(document).is_err());
}
