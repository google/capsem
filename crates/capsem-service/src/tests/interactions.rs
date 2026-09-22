use super::*;

#[tokio::test]
async fn interactions_project_model_messages_tools_and_results_without_cross_call_joins() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/interactions");
    std::fs::create_dir_all(&session).unwrap();
    let db_path = session.join("session.db");
    let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
    writer.shutdown_blocking();
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection.execute_batch(r#"
        INSERT INTO model_calls(id,event_id,timestamp,provider,method,path,trace_id)
        VALUES(1,'aaaaaa000001','2026-09-10T00:00:01Z','openai','POST','/v1/responses','trace-1'),
              (2,'aaaaaa000002','2026-09-10T00:00:02Z','openai','POST','/v1/responses','trace-1'),
              (3,'aaaaaa000003','2026-09-10T00:00:03Z','openai','POST','/v1/responses','other-trace');
        INSERT INTO model_items(event_id,model_call_id,timestamp,provider,path,trace_id,turn_id,
                                kind,item_index,call_id,content,content_hash)
        VALUES('bbbbbb000001',1,'2026-09-10T00:00:01Z','openai','/v1/responses','trace-1','turn-1',
               'request',1,'','{"input":"hello"}','blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'),
              ('bbbbbb000002',1,'2026-09-10T00:00:01Z','openai','/v1/responses','trace-1','turn-1',
               'reasoning',2,'','plan','blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'),
              ('bbbbbb000003',1,'2026-09-10T00:00:01Z','openai','/v1/responses','trace-1','turn-1',
               'response',3,'','null','blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'),
              ('bbbbbb000004',2,'2026-09-10T00:00:02Z','openai','/v1/responses','trace-1','turn-1',
               'tool_response',1,'reused','{"result":[true,null,3]}','blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd');
        INSERT INTO tool_calls(event_id,timestamp,model_call_id,trace_id,turn_id,call_index,call_id,
                               tool_name,origin,server_name,arguments,response_preview,decision,error_message)
        VALUES('cccccc000001','2026-09-10T00:00:01Z',1,'trace-1','turn-1',0,'reused','lookup','model',NULL,
               '{"filter":{"values":[1,true,null]}}',NULL,'allowed',NULL),
              ('cccccc000002','2026-09-10T00:00:04Z',NULL,'mcp-trace',NULL,0,'reused','search','mcp','knowledge',
               'null','{"content":[{"type":"text","text":"found"}]}','allowed',NULL),
              ('cccccc000003','2026-09-10T00:00:05Z',999,'orphan',NULL,0,'bad','bad','builtin','tools',
               '{"broken":',NULL,'error','failed');
        INSERT INTO tool_responses(model_call_id,call_id,content_preview,is_error,trace_id)
        VALUES(2,'reused','{"result":[true,null,3]}',0,'trace-1'),
              (3,'reused','WRONG RESULT',1,'other-trace');
    "#).unwrap();
    drop(connection);
    insert_fake_instance_with_session_dir(&state, "interactions", std::process::id(), session);
    let app = build_service_router(Arc::clone(&state));
    let (status, value) = route_request(app, axum::http::Method::GET, "/vms/interactions/stats/detail", None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let detail: api::VmStatsDetailResponse = serde_json::from_value(value.clone()).unwrap();
    let items = &value["interactions"]["items"];
    assert_eq!(detail.interactions.items.len(), 7);
    let by_id = |id: &str| {
        items
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["event_id"] == id)
            .unwrap()
    };
    let request = by_id("bbbbbb000001");
    assert_eq!(request["model_call_id"], 1);
    assert_eq!(request["model_event_id"], "aaaaaa000001");
    assert_eq!(request["trace_id"], "trace-1");
    assert_eq!(request["turn_id"], "turn-1");
    assert_eq!(request["item_index"], 1);
    assert_eq!(request["content"]["kind"], "request_preview");
    assert_eq!(
        request["content"]["payload"]["content"]["value"],
        json!({"input":"hello"})
    );
    let reasoning = by_id("bbbbbb000002");
    assert_eq!(reasoning["content"]["role"], "assistant");
    assert_eq!(reasoning["content"]["blocks"][0]["kind"], "reasoning");
    assert_eq!(reasoning["content"]["blocks"][0]["payload"]["status"], "unknown");
    assert_eq!(
        by_id("bbbbbb000003")["content"]["blocks"][0]["payload"]["content"],
        json!({"kind":"text","text":"null"})
    );
    let tool = &by_id("cccccc000001")["content"];
    assert_eq!(
        tool["arguments"]["content"]["value"],
        json!({"filter":{"values":[1,true,null]}})
    );
    assert_eq!(
        tool["result"],
        json!(null),
        "a reused call ID must not attach another result"
    );
    let result = &by_id("bbbbbb000004")["content"];
    assert_eq!(result["is_error"], false, "the error flag belongs to this model call");
    assert_eq!(result["payload"]["content"]["value"], json!({"result":[true,null,3]}));
    let mcp = &by_id("cccccc000002")["content"];
    assert_eq!(mcp["origin"], "mcp");
    assert_eq!(mcp["server_name"], "knowledge");
    assert_eq!(mcp["arguments"]["content"], json!({"kind":"json","value":null}));
    assert_eq!(
        mcp["result"]["payload"]["content"]["value"],
        json!({"content":[{"type":"text","text":"found"}]})
    );
    assert_eq!(mcp["result"]["is_error"], json!(null));
    let orphan = by_id("cccccc000003");
    assert_eq!(orphan["model_call_id"], 999);
    assert_eq!(orphan["model_event_id"], json!(null));
    assert_eq!(orphan["content"]["arguments"]["content"]["reason"], "invalid_json");
    assert_eq!(orphan["content"]["result"]["is_error"], true);
    assert_eq!(orphan["content"]["result"]["error_message"], "failed");
    assert!(state.stats_detail_response_cache.lock().unwrap().is_empty());
}

#[tokio::test]
async fn interactions_fail_on_missing_schema_or_corrupt_result_flags() {
    for mutation in [
        "DROP TABLE model_items",
        "INSERT INTO tool_responses(model_call_id,call_id,is_error) VALUES(1,'reused',0),(1,'reused',2)",
    ] {
        let (state, _dir) = make_test_state_with_tempdir();
        let session = state.run_dir.join("sessions/corrupt-interactions");
        std::fs::create_dir_all(&session).unwrap();
        let db_path = session.join("session.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 8).unwrap();
        writer.shutdown_blocking();
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                r#"
            INSERT INTO model_items(event_id,model_call_id,timestamp,provider,path,kind,item_index,call_id,content_hash)
            VALUES('dddddd000001',1,'now','openai','/model','tool_response',1,'reused',
                   'blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
        "#,
            )
            .unwrap();
        connection.execute_batch(mutation).unwrap();
        drop(connection);
        insert_fake_instance_with_session_dir(&state, "corrupt-interactions", std::process::id(), session);
        let (status, body) = route_request(
            build_service_router(state),
            axum::http::Method::GET,
            "/vms/corrupt-interactions/stats/detail",
            None,
        )
        .await;
        assert!(status.is_server_error(), "{mutation}: {status} {body}");
        assert!(body.to_string().contains("stats_detail"), "{body}");
    }
}
