//! Canonical Responses item and content events for every fixture turn.
use bytes::Bytes;
use serde_json::{json, Value};
use std::fmt::Write as _;

pub(super) fn stream(response: Value) -> Bytes {
    let events = events(response);
    let mut body = String::new();
    for event in events {
        write!(body, "event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())
            .expect("writing to a String cannot fail");
    }
    Bytes::from(body)
}

pub(super) fn events(response: Value) -> Vec<Value> {
    let mut events = Vec::new();
    let mut emit = |name: &str, mut data: Value| {
        data["type"] = json!(name);
        data["sequence_number"] = json!(events.len());
        events.push(data);
    };
    let mut initial = response.clone();
    initial["output"] = json!([]);
    initial["status"] = json!("in_progress");
    initial["usage"] = Value::Null;
    emit("response.created", json!({"response": initial}));
    for (index, item) in response["output"].as_array().unwrap().iter().cloned().enumerate() {
        let mut opened = item.clone();
        opened["status"] = json!("in_progress");
        if item["type"] == "message" {
            opened["content"] = json!([]);
        } else if item["type"] == "function_call" {
            opened["arguments"] = json!("");
        } else if item["type"] == "reasoning" {
            opened["summary"] = json!([]);
        } else {
            opened["input"] = json!("");
        }
        emit(
            "response.output_item.added",
            json!({"output_index": index, "item": opened}),
        );
        if item["type"] == "message" {
            let part = item["content"][0].clone();
            let mut start = part.clone();
            start["text"] = json!("");
            emit(
                "response.content_part.added",
                json!({"output_index": index, "content_index": 0, "item_id": item["id"], "part": start}),
            );
            emit(
                "response.output_text.delta",
                json!({"output_index": index, "content_index": 0, "item_id": item["id"], "delta": part["text"], "logprobs": []}),
            );
            emit(
                "response.output_text.done",
                json!({"output_index": index, "content_index": 0, "item_id": item["id"], "text": part["text"], "logprobs": []}),
            );
            emit(
                "response.content_part.done",
                json!({"output_index": index, "content_index": 0, "item_id": item["id"], "part": part}),
            );
        } else if item["type"] == "function_call" {
            emit(
                "response.function_call_arguments.delta",
                json!({"output_index": index, "item_id": item["id"], "delta": item["arguments"]}),
            );
            emit(
                "response.function_call_arguments.done",
                json!({"output_index": index, "item_id": item["id"], "arguments": item["arguments"]}),
            );
        } else if item["type"] == "reasoning" {
            let part = item["summary"][0].clone();
            let mut initial_part = part.clone();
            initial_part["text"] = json!("");
            emit(
                "response.reasoning_summary_part.added",
                json!({"output_index": index, "summary_index": 0, "item_id": item["id"], "part": initial_part}),
            );
            emit(
                "response.reasoning_summary_text.delta",
                json!({"output_index": index, "summary_index": 0, "item_id": item["id"], "delta": part["text"]}),
            );
            emit(
                "response.reasoning_summary_text.done",
                json!({"output_index": index, "summary_index": 0, "item_id": item["id"], "text": part["text"]}),
            );
            emit(
                "response.reasoning_summary_part.done",
                json!({"output_index": index, "summary_index": 0, "item_id": item["id"], "part": part}),
            );
        } else {
            emit(
                "response.custom_tool_call_input.delta",
                json!({"output_index": index, "item_id": item["id"], "delta": item["input"]}),
            );
            emit(
                "response.custom_tool_call_input.done",
                json!({"output_index": index, "item_id": item["id"], "input": item["input"]}),
            );
        }
        emit(
            "response.output_item.done",
            json!({"output_index": index, "item": item}),
        );
    }
    emit("response.completed", json!({"response": response}));
    events
}
