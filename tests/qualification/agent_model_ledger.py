"""Agent model evidence through its declared transport and real body archive."""

import json


def assert_model_ledger(db, archive, expect, token, contains_prompt):
    models = [
        dict(row)
        for row in db.execute(
            "SELECT id, event_id, provider, path, method, model, process_name, tools_count, "
            "status_code, text_content, stop_reason, stream, input_tokens, "
            "output_tokens, trace_id, credential_ref FROM model_calls ORDER BY id"
        )
    ]
    websocket = expect.get("response_transport") == "websocket"
    network_requests = set()
    if websocket:
        for row in db.execute(
            "SELECT event_id, method, status_code, process_name FROM net_events WHERE conn_type='wss-mitm'"
        ):
            assert row["method"] == "GET" and row["status_code"] == 101, dict(row)
            assert row["process_name"] == expect["turn"][0], dict(row)
            body = archive.read(row["event_id"], "net_events", "request")
            assert body is not None, dict(row)
            network_requests.add(body)
    turns, response_ids = [], set()
    for model in models:
        body = archive.read(model["event_id"], "model_calls", "request")
        assert body is not None, model
        request = json.loads(body)
        if websocket:
            assert model["method"] == "GET" and body in network_requests, model
            assert request["type"] == "response.create", request
            assert request.get("generate") is not False, request
        proof = contains_prompt(request, token) or (
            websocket and request.get("previous_response_id") in response_ids
        )
        if not proof:
            continue
        turns.append(model)
        if expect["provider"] == "openai":
            body = archive.read(model["event_id"], "model_calls", "response")
            assert body is not None, model
            if websocket:
                events = [json.loads(line) for line in body.decode().splitlines()]
            else:
                events = [
                    json.loads(line.removeprefix("data: "))
                    for line in body.decode().splitlines()
                    if line.startswith("data: ")
                ]
            assert events[0]["type"] == "response.created", events
            assert events[-1]["type"] == "response.completed", events
            final = events[-1]["response"]
            assert final["status"] == "completed" and final["error"] is None, final
            response_ids.add(final["id"])
            if model["text_content"] == token:
                assert final["output"][0]["content"][0]["text"] == token, final
            if websocket:
                assert model["stream"] == 1, model
                assert model["input_tokens"] == final["usage"]["input_tokens"] == 31
                assert model["output_tokens"] == final["usage"]["output_tokens"] == 17
    assert turns, models
    if websocket:
        assert len(models) == len(turns) == 2, models
        assert_custom_tool_lifecycle(db, archive, turns, token)
    assert all(
        model["provider"] == expect["provider"]
        and model["path"] == expect["path"]
        and model["method"] == ("GET" if websocket else "POST")
        and model["model"]
        and model["process_name"] == expect["turn"][0]
        and model["status_code"] == (101 if websocket else 200)
        for model in turns
    ), turns
    assert any(
        model["text_content"] == token and model["stop_reason"] == "end_turn"
        for model in turns
    ), turns


def assert_custom_tool_lifecycle(db, archive, turns, token):
    first, last = turns
    assert first["stop_reason"] == "tool_use", first
    assert first["trace_id"] and first["trace_id"] == last["trace_id"], turns
    calls = [
        dict(row)
        for row in db.execute(
            "SELECT * FROM tool_calls WHERE model_call_id=?", (first["id"],)
        )
    ]
    assert len(calls) == 1, calls
    call = calls[0]
    assert call["tool_name"] == "functions.exec" and call["origin"] == "native", call
    assert call["call_id"] == "call_capsem_mcp_echo", call
    assert call["transport"] == "websocket" and call["provider"] == "openai", call
    assert call["status"] == "observed" and call["decision"] == "allowed", call
    assert call["trace_id"] == call["turn_id"] == first["trace_id"], call
    assert token in call["arguments"] and "await tools[" in call["arguments"], call
    results = [
        dict(row)
        for row in db.execute(
            "SELECT * FROM tool_responses WHERE model_call_id=?", (last["id"],)
        )
    ]
    assert len(results) == 1, results
    result = results[0]
    assert set(result) == {
        "id",
        "event_id",
        "model_call_id",
        "call_id",
        "content_preview",
        "is_error",
        "trace_id",
        "turn_id",
        "credential_ref",
    }, result
    assert result["call_id"] == call["call_id"] and result["is_error"] == 0, result
    assert result["trace_id"] == result["turn_id"] == first["trace_id"], result
    assert result["credential_ref"] == last["credential_ref"], result
    body = archive.read(result["event_id"], "tool_responses", "response")
    assert body is not None, result
    assert body.decode() == result["content_preview"], result
    assert body.decode().startswith("Script completed\n"), result
    assert body.decode().splitlines()[-1] == token, result
