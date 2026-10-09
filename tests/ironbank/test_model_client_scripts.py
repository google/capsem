import ast
import json
import subprocess
import sys
from pathlib import Path

import pytest
from helpers.bounded import bounded
from ironbank.model_client_config import (
    HERMETIC_AGY_MODEL_DISPLAY,
    HERMETIC_ANTHROPIC_MODEL,
    HERMETIC_GEMINI_MODEL,
)
from ironbank.model_client_scripts import (
    agy_cli_script,
    claude_ollama_launch_script,
    codex_cli_script,
    codex_ollama_launch_script,
    gemini_api_script,
    openai_responses_api_script,
    openai_two_tool_calls_script,
)

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _assigned_value(script: str, name: str, environment: dict):
    assignments = [
        node for node in ast.walk(ast.parse(script))
        if isinstance(node, ast.Assign)
        and any(isinstance(target, ast.Name) and target.id == name for target in node.targets)
    ]
    assert len(assignments) == 1
    # Execute the generated client's actual selection/expression without its
    # DNS, filesystem or subprocess setup. The surrounding fixtures stay real.
    probe = (
        "import json,sys\n"
        "globals().update(json.loads(sys.argv[1]))\n"
        + ast.unparse(assignments[0]) + "\n"
        + f"print(json.dumps({name}))"
    )
    result = subprocess.run(
        bounded([sys.executable, "-c", probe, json.dumps(environment)], 10),
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)


@pytest.mark.parametrize("render", [openai_responses_api_script, openai_two_tool_calls_script])
def test_streaming_client_uses_completed_tool_arguments(render) -> None:
    completed = {"type": "function_call", "call_id": "call-1", "name": "exec_command", "arguments": '{"cmd":"true"}'}
    events = [
        {"type": "response.output_item.added", "item": {**completed, "arguments": ""}},
        {"type": "response.function_call_arguments.delta", "delta": completed["arguments"]},
        {"type": "response.output_item.done", "item": completed},
    ]
    assert _assigned_value(render("https://api.openai.com"), "tool_item", {"first_events": events}) == completed


@pytest.mark.parametrize("render,key", [
    (codex_cli_script, "cmd"), (codex_ollama_launch_script, "cmd"),
    (claude_ollama_launch_script, "command"), (agy_cli_script, "CommandLine"),
])
def test_native_client_expected_arguments_keep_the_fixture_shell_quotes(render, key) -> None:
    arguments = _assigned_value(render("http://fixture"), "call_args", {"NONCE": "nonce", "TARGET": "/workspace/file", "WORKSPACE": "/workspace"})
    assert arguments[key] == r"printf '%s\n' 'nonce' > '/workspace/file'"


def test_gemini_replay_uses_release_target_model() -> None:
    script = gemini_api_script("https://generativelanguage.googleapis.com")

    assert HERMETIC_GEMINI_MODEL == "gemini-3.5-flash"
    assert "gemini-3.5-flash" in script
    assert "gemini-2.5-flash" not in script


def test_anthropic_replay_uses_release_target_model() -> None:
    sdk_test = PROJECT_ROOT / "tests" / "ironbank" / "test_model_sdk_ledger.py"
    mock_server = PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "anthropic.rs"

    assert HERMETIC_ANTHROPIC_MODEL == "claude-sonnet-4-6"
    sdk_text = sdk_test.read_text(encoding="utf-8")
    mock_text = mock_server.read_text(encoding="utf-8")
    assert "HERMETIC_ANTHROPIC_MODEL" in sdk_text
    assert HERMETIC_ANTHROPIC_MODEL in mock_text
    for path, text in ((sdk_test, sdk_text), (mock_server, mock_text)):
        assert "claude-sonnet-4-20250514" not in text, path


def test_agy_noninteractive_script_selects_model_explicitly() -> None:
    script = agy_cli_script("http://127.0.0.1:3713")

    assert '"agy",' in script
    assert '"--model",' in script
    assert f'HERMETIC_AGY_MODEL_DISPLAY = "{HERMETIC_AGY_MODEL_DISPLAY}"' in script
    assert 'emit_result("google", "daily-cloudcode-pa.googleapis.com", "/v1internal:streamGenerateContent"' in script
    assert '"run_command"' in script
    assert '"CommandLine": "printf' in script
    assert '"/api/chat"' not in script
    assert '"write_to_file"' not in script
