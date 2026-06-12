from __future__ import annotations

import importlib.util
import json
from pathlib import Path

from helpers.mock_server import start_mock_server, stop_process

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RECORDER_PATH = PROJECT_ROOT / "scripts" / "protocol_fixture_recorder.py"


def _load_recorder():
    spec = importlib.util.spec_from_file_location("protocol_fixture_recorder", RECORDER_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_protocol_fixture_recorder_uses_mock_server_and_sanitizes(tmp_path):
    recorder = _load_recorder()
    proc = None
    try:
        proc, ready = start_mock_server()
        written = recorder.record_mock_server(ready["base_url"], tmp_path)
    finally:
        stop_process(proc)

    names = {path.stem for path in written}
    assert {
        "anthropic_claude_messages",
        "openai_codex_chat_completions",
        "gemini_agy_generate_content",
        "ollama_openai_chat_completions",
        "oauth_token_exchange",
        "mcp_tools_list",
        "mcp_tool_call",
        "credential_response_capture",
    }.issubset(names)

    combined = "\n".join(path.read_text() for path in written)
    assert "capsem_test_" not in combined
    assert "credential:blake3:" in combined

    for path in written:
        payload = json.loads(path.read_text())
        fixture = recorder.ProtocolFixture.model_validate(payload)
        assert fixture.schema_ == "capsem.protocol_fixture.v1"
        assert fixture.client.name
        assert fixture.client.version
        assert fixture.protocol_family in {
            "http",
            "model",
            "mcp",
            "oauth",
            "credential",
        }
        assert fixture.auth_mode in {"none", "bearer", "api_key", "oauth_code"}
        assert fixture.expected_ledger_rows
        assert fixture.expected_visible_bytes >= 0


def test_protocol_fixture_replay_covers_recorded_flows(tmp_path):
    recorder = _load_recorder()
    proc = None
    try:
        proc, ready = start_mock_server()
        written = recorder.record_mock_server(ready["base_url"], tmp_path)
        results = recorder.replay_fixtures(ready["base_url"], written)
    finally:
        stop_process(proc)

    assert {result.name for result in results} == {path.stem for path in written}
    assert all(result.status_matches for result in results)
    assert all(result.visible_bytes_match for result in results)
    assert {
        result.protocol_family for result in results
    } == {"model", "oauth", "mcp", "credential"}
