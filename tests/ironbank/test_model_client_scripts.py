from pathlib import Path

from ironbank.model_client_config import (
    HERMETIC_AGY_MODEL_DISPLAY,
    HERMETIC_ANTHROPIC_MODEL,
    HERMETIC_GEMINI_MODEL,
)
from ironbank.model_client_scripts import agy_cli_script, gemini_api_script


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_gemini_replay_uses_release_target_model() -> None:
    script = gemini_api_script("https://generativelanguage.googleapis.com")

    assert HERMETIC_GEMINI_MODEL == "gemini-3.5-flash"
    assert "gemini-3.5-flash" in script
    assert "gemini-2.5-flash" not in script


def test_anthropic_replay_uses_release_target_model() -> None:
    sdk_test = PROJECT_ROOT / "tests" / "ironbank" / "test_model_sdk_ledger.py"
    mock_server = PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "main.rs"

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
