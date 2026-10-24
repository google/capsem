//! AGY1.1.3 lazy dispatch, from the original known-synthetic native request.
//! A final token requires the exact original call and typed successful MCP result.
use bytes::Bytes;
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const MARKER: &str = "CAPSEM_MCP_PROOF=";
const DISPATCH: &str = "call_mcp_tool";
const CALL_ID: &str = "capsem_google_mcp_echo";
const REQUIRED: [&str; 5] = ["ServerName", "ToolName", "Arguments", "toolSummary", "toolAction"];

pub(super) fn reply(payload: &Value) -> Option<Result<Bytes, String>> {
    // Checkpoint/title calls are distinct; tool metadata is never a user prompt.
    if payload["requestType"] == "checkpoint" {
        return None;
    }
    let request = payload.get("request")?;
    let contents = request.get("contents")?.as_array()?;
    let prompt = contents
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["parts"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .map(|text| {
            text.trim_start()
                .strip_prefix("<USER_REQUEST>")
                .unwrap_or(text)
                .trim_start()
        })
        .find_map(|text| text.strip_prefix(MARKER))?;
    Some(proof(payload, request, contents, prompt).map_err(str::to_owned))
}

fn proof(payload: &Value, request: &Value, contents: &[Value], prompt: &str) -> Result<Bytes, &'static str> {
    let token = prompt.split('.').next().unwrap_or("");
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Google MCP proof needs a UUID hex token");
    }
    let declared = request["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["functionDeclarations"].as_array())
        .flatten()
        .find(|tool| tool["name"] == DISPATCH)
        .ok_or("Google MCP proof needs its declared lazy dispatcher")?;
    let schema = &declared["parameters"];
    let required = schema["required"]
        .as_array()
        .ok_or("Google MCP dispatcher required fields missing")?;
    if schema["type"] != "OBJECT"
        || REQUIRED
            .iter()
            .any(|field| !required.iter().any(|value| value == field))
        || REQUIRED.iter().any(|field| schema["properties"].get(field).is_none())
        || ["ServerName", "ToolName", "toolSummary", "toolAction"]
            .iter()
            .any(|field| schema["properties"][field]["type"] != "STRING")
    {
        return Err("Google MCP dispatcher does not match the captured schema");
    }
    if !advertises_echo(request) {
        return Err("Google MCP proof needs the advertised capsem/local__echo tool");
    }
    let args = json!({"ServerName":"capsem","ToolName":"local__echo","Arguments":{"text":token},
        "toolSummary":"MCP echo proof","toolAction":"Calling echo"});
    let original = json!({"name":DISPATCH,"id":CALL_ID,"args":args});
    let parts: Vec<_> = contents
        .iter()
        .filter_map(|message| message["parts"].as_array())
        .flatten()
        .collect();
    let calls: Vec<_> = parts.iter().filter_map(|part| part.get("functionCall")).collect();
    let results: Vec<_> = parts.iter().filter_map(|part| part.get("functionResponse")).collect();
    if results.is_empty() {
        if !calls.is_empty() {
            return Err("Google MCP original call is missing its result");
        }
        let first = chunk(
            payload,
            json!([{"thoughtSignature":"capsem-agy-fixture-signature","functionCall":original}]),
            false,
        );
        let stop = chunk(payload, json!([{"text":""}]), true);
        return Ok(Bytes::from(format!("data: {first}\n\ndata: {stop}\n\n")));
    }
    if calls.len() != 1 || results.len() != 1 || calls[0] != &original {
        return Err("Google MCP proof needs exactly its original declared call and result");
    }
    let result = results[0];
    if result["name"] != DISPATCH || result["id"] != CALL_ID {
        return Err("Google MCP proof result does not belong to its original call");
    }
    let timestamp_wrapper = result["response"]["output"]
        .as_str()
        .is_some_and(|text| text.starts_with("Created At: "));
    // Actual AGY renders this timestamp wrapper with role=model; the typed
    // Google controls use role=user. Bind each exact form, not role aliases.
    let result_role = if timestamp_wrapper { "model" } else { "user" };
    let pair = contents.get(contents.len().saturating_sub(2)..).unwrap_or(&[]);
    if pair.len() != 2
        || pair[0]["role"] != "model"
        || pair[1]["role"] != result_role
        || !pair[0]["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part.get("functionCall") == Some(&original)))
        || !pair[1]["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part.get("functionResponse") == Some(result)))
    {
        return Err("Google MCP result must follow its original model call");
    }
    if !result["response"]
        .as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("output"))
    {
        return Err("Google MCP result has an unknown or failed response envelope");
    }
    let output = &result["response"]["output"];
    // AGY1.1.3's actual successful native result has a strict timestamp
    // wrapper. The canonical typed MCP controls remain valid too; prose,
    // errors or token substrings are never successful echo results.
    let echoed = match output.as_str() {
        Some(text) if text.starts_with("Created At: ") => timestamped_echo(text, token),
        Some(text) => serde_json::from_str::<Value>(text).is_ok_and(|value| typed_echo(&value, token)),
        None => typed_echo(output, token),
    };
    if !echoed {
        return Err("Google MCP proof needs the exact successful echo result");
    }
    let last = chunk(payload, json!([{"text":token}]), true);
    Ok(Bytes::from(format!("data: {last}\n\n")))
}

fn typed_echo(output: &Value, token: &str) -> bool {
    output.is_object() && output["isError"] == false && output["content"] == json!([{"type":"text","text":token}])
}

fn timestamped_echo(output: &str, token: &str) -> bool {
    let Some((created, rest)) = output
        .strip_prefix("Created At: ")
        .and_then(|text| text.split_once('\n'))
    else {
        return false;
    };
    let Some((completed, payload)) = rest
        .strip_prefix("Completed At: ")
        .and_then(|text| text.split_once('\n'))
    else {
        return false;
    };
    if payload != token {
        return false;
    }
    let (Ok(created), Ok(completed)) = (
        OffsetDateTime::parse(created, &Rfc3339),
        OffsetDateTime::parse(completed, &Rfc3339),
    ) else {
        return false;
    };
    created <= completed
}

fn advertises_echo(request: &Value) -> bool {
    request["systemInstruction"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .any(|text| {
            text.split("<mcp_servers>")
                .skip(1)
                .filter_map(|tail| tail.split_once("</mcp_servers>"))
                .any(|(block, _)| {
                    let mut lines = block.lines().skip_while(|line| *line != "# capsem").skip(1);
                    let section: Vec<_> = lines.by_ref().take_while(|line| !line.starts_with("# ")).collect();
                    section.windows(2).any(|pair| pair == ["Lazy:", "local__echo"])
                })
        })
}

fn chunk(payload: &Value, parts: Value, stop: bool) -> Value {
    let mut candidate = json!({"content":{"parts":parts,"role":"model"}});
    if stop {
        candidate["finishReason"] = json!("STOP");
    }
    json!({"response":{"candidates":[candidate],"usageMetadata":{
        "promptTokenCount":31,"candidatesTokenCount":17,"thoughtsTokenCount":2,"totalTokenCount":50},
        "modelVersion":payload["model"],"responseId":"agy_mcp_proof"},"traceId":"trace_mcp_proof","metadata":{}})
}
