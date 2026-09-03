//! Text-table rendering, pagination, and JSON output contracts for the change and snapshot listings.

use super::*;

#[test]
fn changes_returns_text_table() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("new.txt"), "created").unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Default format is text table, not JSON.
    assert!(
        serde_json::from_str::<Vec<Value>>(&text).is_err(),
        "default response should NOT be a JSON array"
    );
    assert!(text.contains("Changed Files"), "missing header: {text}");
    assert!(text.contains("Path"), "missing Path column: {text}");
    assert!(text.contains("Op"), "missing Op column: {text}");
    assert!(text.contains("new.txt"), "missing file entry: {text}");
    assert!(text.contains("created"), "missing op value: {text}");
}

#[test]
fn changes_pagination_truncates_large_output() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Take empty snapshot, then create 300 files.
    sched.take_snapshot().unwrap();
    for i in 0..300 {
        std::fs::write(ws.join(format!("file_{i:04}.txt")), format!("content {i}")).unwrap();
    }

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Response should be bounded by DEFAULT_MAX_LENGTH + header overhead.
    let max_allowed = crate::mcp::builtin_tools::DEFAULT_MAX_LENGTH as usize + 500;
    assert!(
        text.len() <= max_allowed,
        "response too large: {} chars (max {})",
        text.len(),
        max_allowed
    );
    // Should indicate pagination is available.
    assert!(text.contains("start_index="), "missing pagination hint: {text}");
}

#[test]
fn changes_pagination_continuation() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    for i in 0..300 {
        std::fs::write(ws.join(format!("file_{i:04}.txt")), format!("content {i}")).unwrap();
    }

    // First page.
    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let page1 = extract_text(&resp);

    // Extract start_index from pagination hint.
    let idx_str = page1
        .split("start_index=")
        .nth(1)
        .unwrap()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap();
    let next_start: u64 = idx_str.parse().unwrap();

    // Second page.
    let args = serde_json::json!({"start_index": next_start});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let page2 = extract_text(&resp);

    // Pages should have different content.
    assert_ne!(page1, page2, "pages should differ");
    // Page 2 should not re-include the header.
    assert!(
        !page2.starts_with("Changed Files"),
        "page 2 should not repeat the header"
    );
}

#[test]
fn changes_custom_max_length() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    for i in 0..20 {
        std::fs::write(ws.join(format!("f_{i}.txt")), "x").unwrap();
    }

    let args = serde_json::json!({"max_length": 200});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Header + chunk: allow some overhead for the pagination hint itself.
    assert!(
        text.len() <= 500,
        "response should be short with max_length=200, got {} chars",
        text.len()
    );
    assert!(text.contains("start_index="), "should paginate at max_length=200");
}

#[test]
fn changes_small_result_no_pagination() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("only.txt"), "small").unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    assert!(
        !text.contains("start_index="),
        "should not paginate small results: {text}"
    );
    assert!(text.contains("only.txt"), "missing file entry: {text}");
}

#[test]
fn changes_format_json_returns_raw() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("a.txt"), "original").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("b.txt"), "new").unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_changed_files(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // format=json should return valid JSON array.
    let changes: Vec<Value> = serde_json::from_str(&text).expect("format=json should return valid JSON array");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "b.txt");
    assert_eq!(changes[0]["op"], "created");
}

#[test]
fn list_returns_text_table() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("hello.txt"), "modified world content").unwrap();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // Default format is text table, not JSON.
    assert!(
        serde_json::from_str::<Value>(&text).is_err(),
        "default response should NOT be JSON"
    );
    assert!(text.contains("Snapshots"), "missing header: {text}");
    assert!(text.contains("Checkpoint"), "missing Checkpoint column: {text}");
    // Changes should use compact count columns.
    assert!(text.contains("Created"), "missing Created column: {text}");
    assert!(text.contains("Edited"), "missing Edited column: {text}");
    assert!(text.contains("Deleted"), "missing Deleted column: {text}");
    assert!(
        text.contains("1       "),
        "changes should render numeric compact counts: {text}"
    );
}

#[test]
fn list_pagination_works() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    // Create many snapshots with files to generate a large response.
    for i in 0..8 {
        for j in 0..20 {
            std::fs::write(ws.join(format!("f_{i}_{j}.txt")), format!("{i}{j}")).unwrap();
        }
        sched.take_snapshot().unwrap();
    }

    let args = serde_json::json!({"max_length": 500});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    assert!(
        text.len() <= 1000,
        "response should respect max_length, got {} chars",
        text.len()
    );
    assert!(text.contains("start_index="), "should paginate: {text}");
}

#[test]
fn list_format_json_returns_raw() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("a.txt"), "data").unwrap();
    sched.take_snapshot().unwrap();

    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    // format=json should return valid JSON.
    let summary: Value = serde_json::from_str(&text).expect("format=json should return valid JSON");
    assert!(summary["snapshots"].is_array());
}

#[test]
fn list_format_json_large_payload_is_not_prefixed_with_pagination_text() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    for i in 0..10 {
        for j in 0..80 {
            std::fs::write(ws.join(format!("large_{i}_{j}.txt")), format!("payload {i} {j}")).unwrap();
        }
        sched.take_snapshot().unwrap();
    }

    let args = serde_json::json!({"format": "json", "max_length": 200});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));
    let text = extract_text(&resp);

    assert!(
        !text.starts_with("Content length:"),
        "format=json must not be prefixed with prose pagination: {text}"
    );
    let summary: Value = serde_json::from_str(&text).expect("format=json should return valid JSON");
    assert!(summary["snapshots"].as_array().unwrap().len() >= 10);
    for snap in summary["snapshots"].as_array().unwrap() {
        assert!(
            snap["changes"].is_null(),
            "format=json should stay compact unless include_changes=true: {snap}"
        );
        assert!(
            snap["changes_summary"].is_object(),
            "format=json should include compact change summary: {snap}"
        );
    }
}

/// Contract test: verifies the exact response shape the frontend depends on.
///
/// The frontend (api.ts:listSnapshots) calls callMcpTool('snapshots_list', {format:'json'})
/// and parses result.content[0].text as JSON expecting these fields. If this test
/// breaks, the snapshot panel will break too.
#[test]
fn list_format_json_frontend_contract() {
    let (_tmp, session, mut sched) = setup();
    let ws = session.join("workspace");

    std::fs::write(ws.join("hello.txt"), "world").unwrap();
    sched.take_snapshot().unwrap();
    std::fs::write(ws.join("hello.txt"), "changed").unwrap();
    sched.take_snapshot().unwrap();

    // Frontend always passes format: "json".
    let args = serde_json::json!({"format": "json"});
    let resp = handle_list_snapshots(&args, &sched, &ws, Some(serde_json::json!(1)));

    // Response must have result.content[0].text.
    let result = resp.result.as_ref().expect("response must have result");
    let content = result["content"].as_array().expect("result must have content array");
    assert!(!content.is_empty(), "content must not be empty");
    let text = content[0]["text"].as_str().expect("content[0] must have text string");

    // text must be valid JSON with the expected shape.
    let data: Value = serde_json::from_str(text).expect("content text must be valid JSON when format=json");

    // Top-level fields the frontend depends on.
    assert!(data["snapshots"].is_array(), "must have snapshots array");
    assert!(data["auto_max"].is_number(), "must have auto_max number");
    assert!(data["manual_max"].is_number(), "must have manual_max number");
    assert!(
        data["manual_available"].is_number(),
        "must have manual_available number"
    );

    // Each snapshot must have the fields SnapshotsTab.svelte reads.
    let snaps = data["snapshots"].as_array().unwrap();
    assert!(snaps.len() >= 2, "should have at least 2 snapshots");
    for snap in snaps {
        assert!(snap["checkpoint"].is_string(), "snapshot must have checkpoint: {snap}");
        assert!(snap["slot"].is_number(), "snapshot must have slot: {snap}");
        assert!(snap["origin"].is_string(), "snapshot must have origin: {snap}");
        // name and hash can be null.
        assert!(snap["age"].is_string(), "snapshot must have age: {snap}");
        assert!(
            snap["files_count"].is_number(),
            "snapshot must have files_count: {snap}"
        );
        assert!(
            snap["changes_summary"].is_object(),
            "snapshot must have compact changes_summary object: {snap}"
        );
        assert!(
            snap["changes"].is_null(),
            "full changes must require include_changes=true: {snap}"
        );
    }
}
