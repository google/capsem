use super::*;
use crate::environment::{EnvironmentMcpCatalogInput, EnvironmentMcpDefinition};
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

#[test]
fn catalog_mcp_configures_only_declared_sandbox_values_and_binding_targets() {
    let template: McpCatalogDefinition = serde_json::from_value(json!({
        "kind":"sandbox","executable":"review","args":["--stdio"],"workdir":"/workspace",
        "variables":{"WORKSPACE":{"description":"Workspace","required":true}},
        "secret_variables":{"REVIEW_TOKEN":{"description":"Review token","required":true}}
    }))
    .unwrap();
    let input = || {
        json!({
            "values":{"WORKSPACE":"project"},
            "secrets":{"token":{"binding_id":"00000000-0000-4000-8000-000000000010","env_name":"REVIEW_TOKEN"}}
        })
    };
    let parse = |value| serde_json::from_value::<EnvironmentMcpCatalogInput>(value).unwrap();
    let definition = template.clone().configure(parse(input()), true).unwrap();
    let EnvironmentMcpDefinition::Sandbox {
        executable,
        args,
        workdir,
        variables,
        secrets,
    } = definition
    else {
        panic!("sandbox template changed kind")
    };
    assert_eq!(executable, "review");
    assert_eq!(args, ["--stdio"]);
    assert_eq!(workdir.as_deref(), Some("/workspace"));
    assert_eq!(variables["WORKSPACE"], "project");
    assert_eq!(secrets["token"].env_name.as_deref(), Some("REVIEW_TOKEN"));
    let mut wrong = input();
    wrong["secrets"]["token"]["env_name"] = "OTHER_TOKEN".into();
    assert!(template.clone().configure(parse(wrong), true).is_err());
    let mut wrong = input();
    wrong["values"]["REVIEW_TOKEN"] = "undeclared secret value".into();
    assert!(template.clone().configure(parse(wrong), true).is_err());
    assert!(template.configure(parse(input()), false).is_err());
}
