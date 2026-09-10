"""Typed payloads preserve tool-defined JSON and distinguish capture evidence."""

import json

import pytest
from capsem import models
from pydantic import TypeAdapter, ValidationError


@pytest.mark.parametrize("value", [None, True, 4, 1.5, "text", [], {"nested": [None, {"x": True}]}])
def test_arbitrary_json_has_native_typed_values(value: object) -> None:
    wire = {"status": "unknown", "content": {"kind": "json", "value": value}}
    payload = models.CapturedPayload.model_validate_json(json.dumps(wire))
    assert payload.status is models.CaptureStatus.UNKNOWN
    assert isinstance(payload.content, models.JsonContent)
    assert payload.content.value == value
    assert json.loads(payload.model_dump_json()) == wire


@pytest.mark.parametrize("content", [
    {"kind": "json"}, {"kind": "invented", "value": 1},
    {"kind": "text", "text": "x", "value": 2},
    {"kind": "raw", "raw": "{", "reason": "invented"},
])
def test_invalid_payload_shapes_fail(content: object) -> None:
    with pytest.raises(ValidationError):
        TypeAdapter(models.CapturedContent).validate_json(json.dumps(content))


def test_messages_and_mcp_calls_have_distinct_objects() -> None:
    wire = {"kind": "tool_call", "call_id": "call-1", "tool_name": "search",
            "server_name": "knowledge", "origin": "mcp", "decision": "allowed",
            "arguments": {"status": "unknown", "content": {"kind": "json", "value": {"q": "SDK"}}},
            "result": {"kind": "tool_result", "call_id": "call-1", "is_error": None,
                       "error_message": None, "payload": {"status": "truncated", "content": {
                           "kind": "raw", "raw": '{"content":[', "reason": "truncated"}}}}
    content = TypeAdapter(models.InteractionContent).validate_json(json.dumps(wire))
    assert isinstance(content, models.InteractionToolCall)
    assert content.origin is models.ToolOrigin.MCP
    assert content.result is not None and content.result.payload is not None
    assert isinstance(content.result.payload.content, models.RawContent)
    assert content.result.payload.content.raw == '{"content":['
    assert content.result.is_error is None
    message = TypeAdapter(models.InteractionContent).validate_json(json.dumps({
        "kind": "message", "role": "assistant", "blocks": [{"kind": "reasoning", "payload": None}],
    }))
    assert isinstance(message, models.InteractionMessage)
    assert message.blocks[0].kind is models.InteractionBlockKind.REASONING


@pytest.mark.parametrize("number", ["NaN", "Infinity", "-Infinity"])
def test_non_json_numbers_are_rejected_even_when_nested(number: str) -> None:
    with pytest.raises(ValidationError):
        models.JsonContent.model_validate_json('{"kind":"json","value":{"nested":[' + number + ']}}')
