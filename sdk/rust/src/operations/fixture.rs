use std::sync::LazyLock;

use serde_json::{json, Map, Value};

use crate::test_gateway::Server;

static CONTRACT: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../../specification/openapi.json")).unwrap());

fn sample(schema: &Value, full: bool) -> Value {
    if let Some(reference) = schema["$ref"].as_str() {
        return sample(CONTRACT.pointer(&reference[1..]).unwrap(), full);
    }
    if let Some(variants) = schema["oneOf"].as_array() {
        return sample(variants.iter().find(|s| s["type"] != "null").unwrap(), full);
    }
    let kind = schema["type"]
        .as_str()
        .or_else(|| {
            schema["type"]
                .as_array()
                .and_then(|types| types.iter().filter_map(Value::as_str).find(|t| *t != "null"))
        })
        .unwrap();
    match kind {
        "object" => {
            let properties = schema["properties"].as_object().cloned().unwrap_or_default();
            Value::Object(
                properties
                    .iter()
                    .filter(|(key, _)| {
                        full || schema["required"]
                            .as_array()
                            .is_some_and(|fields| fields.contains(&json!(key)))
                    })
                    .map(|(key, schema)| (key.clone(), sample(schema, full)))
                    .collect(),
            )
        }
        "array" => json!([]), // Recursive file children remain bounded.
        "string" if schema["enum"].is_array() => schema["enum"][0].clone(),
        "string" => json!("sample /?&é"),
        "integer" => json!(3),
        "number" => json!(3.5),
        "boolean" => json!(true),
        "null" => Value::Null,
        _ => panic!("unsupported contract fixture: {schema}"),
    }
}

pub struct Case {
    pub input: Value,
    pub response: Value,
    pub binary: bool,
    path: String,
    method: String,
    query: Vec<(String, String)>,
    request_media: Option<String>,
}

impl Case {
    pub fn operation_ids() -> std::collections::BTreeSet<String> {
        CONTRACT["paths"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|methods| methods.as_object().unwrap().values())
            .map(|operation| operation["operationId"].as_str().unwrap().to_owned())
            .collect()
    }

    pub fn new(id: &str, full: bool) -> Self {
        let (path, method, operation) = CONTRACT["paths"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(path, methods)| {
                methods
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(move |(method, op)| (path, method, op))
            })
            .find(|(_, _, op)| op["operationId"] == id)
            .unwrap();
        let mut input = Map::new();
        let mut url = reqwest::Url::parse("http://localhost").unwrap();
        let mut query = Vec::new();
        let parameters = operation["parameters"].as_array().cloned().unwrap_or_default();
        for parameter in &parameters {
            if !full && parameter["required"] != true {
                continue;
            }
            let name = parameter["name"].as_str().unwrap();
            let mut value = sample(&parameter["schema"], full);
            if parameter["schema"]["type"] == "array" {
                value = json!(["net", "model"]);
            }
            let wire = match &value {
                Value::String(text) => text.clone(),
                Value::Array(values) => values.iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>().join(","),
                value => value.to_string(),
            };
            if parameter["in"] == "query" {
                query.push((name.to_owned(), wire));
            }
            input.insert(name.into(), value);
        }
        {
            let mut segments = url.path_segments_mut().unwrap();
            segments.clear();
            for segment in path[1..].split('/') {
                segments.push(if segment.starts_with('{') {
                    input[&segment[1..segment.len() - 1]].as_str().unwrap()
                } else {
                    segment
                });
            }
        }
        let request_media = operation["requestBody"]["content"].as_object().map(|content| {
            let (media, body) = content.iter().next().unwrap();
            input.insert(
                "body".into(),
                if media == "application/octet-stream" {
                    json!([0, 255, 13, 10])
                } else {
                    sample(&body["schema"], full)
                },
            );
            media.clone()
        });
        let content = &operation["responses"]["200"]["content"];
        let binary = content.get("application/octet-stream").is_some();
        let response = if binary {
            Value::Null
        } else {
            sample(&content["application/json"]["schema"], full)
        };
        Self {
            input: Value::Object(input),
            response,
            binary,
            path: url.path().into(),
            method: method.to_uppercase(),
            query,
            request_media,
        }
    }

    pub async fn server(&self, status: u16, invalid: Option<&[u8]>) -> Server {
        let body = if self.binary {
            vec![0, 255, 13, 10]
        } else {
            serde_json::to_vec(&self.response).unwrap()
        };
        Server::reply(status, invalid.unwrap_or(&body), None).await
    }

    pub async fn assert_request(&self, server: &mut Server) {
        let (parts, body) = server.received.recv().await.unwrap();
        assert_eq!(parts.method.as_str(), self.method);
        assert_eq!(parts.uri.path(), self.path);
        let url = reqwest::Url::parse(&format!("http://localhost{}", parts.uri)).unwrap();
        assert_eq!(url.query_pairs().into_owned().collect::<Vec<_>>(), self.query);
        assert_eq!(parts.headers["authorization"], "Bearer private-token");
        assert_eq!(
            parts.headers["accept"],
            if self.binary {
                "application/octet-stream"
            } else {
                "application/json"
            }
        );
        match self.request_media.as_deref() {
            Some("application/octet-stream") => assert_eq!(body, vec![0, 255, 13, 10]),
            Some(media) => {
                assert_eq!(parts.headers["content-type"], media);
                let actual: Value = serde_json::from_slice(&body).unwrap();
                for (key, value) in self.input["body"].as_object().unwrap() {
                    assert_eq!(&actual[key], value, "request field {key}");
                }
            }
            None => assert!(body.is_empty()),
        }
        assert!(server.received.try_recv().is_err());
    }
}
