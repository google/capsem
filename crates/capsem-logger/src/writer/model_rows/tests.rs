use super::*;
use crate::{events::ToolCallEntry, DbWriter, WriteOp};

#[test]
fn model_tool_rows_preserve_websocket_and_existing_http_transports() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let writer = DbWriter::open(&path, 16).unwrap();
    let cases = [
        ("GET", 101, true, "websocket"),
        ("POST", 200, true, "sse"),
        ("POST", 200, false, "http"),
    ];
    for (index, (method, status, stream, _)) in cases.iter().enumerate() {
        writer.write_blocking(WriteOp::ModelCall(ModelCall {
            event_id: None,
            timestamp: std::time::SystemTime::now(),
            provider: "openai".into(),
            protocol: Some("openai".into()),
            model: Some("gpt-fixture".into()),
            process_name: Some("codex".into()),
            pid: None,
            method: (*method).into(),
            path: "/v1/responses".into(),
            stream: *stream,
            system_prompt_preview: None,
            messages_count: 1,
            tools_count: 1,
            request_bytes: 0,
            request_body: None,
            message_id: None,
            status_code: Some(*status),
            text_content: None,
            thinking_content: None,
            response_body: None,
            stop_reason: Some("tool_use".into()),
            input_tokens: Some(31),
            output_tokens: Some(17),
            usage_details: Default::default(),
            duration_ms: 1,
            response_bytes: 0,
            estimated_cost_usd: 0.0,
            trace_id: Some(format!("trace_{index}")),
            credential_ref: None,
            tool_responses: vec![],
            tool_calls: vec![ToolCallEntry {
                event_id: None,
                call_index: 0,
                call_id: format!("call_{index}"),
                tool_name: "functions.exec".into(),
                arguments: Some("await tools.echo()".into()),
                origin: "native".into(),
                trace_id: None,
            }],
        }));
    }
    writer.shutdown_blocking();
    let db = Connection::open(&path).unwrap();
    let rows: Vec<(String, String, String)> = db
        .prepare("SELECT transport,call_id,trace_id FROM tool_calls ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 3);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.0, cases[index].3);
        assert_eq!(row.1, format!("call_{index}"));
        assert_eq!(row.2, format!("trace_{index}"));
    }
}
