from ironbank.model_client_config import HERMETIC_AGY_MODEL_DISPLAY, HERMETIC_GEMINI_MODEL
from ironbank.model_client_scripts import agy_cli_script, gemini_api_script


def test_gemini_replay_uses_release_target_model() -> None:
    script = gemini_api_script("https://generativelanguage.googleapis.com")

    assert HERMETIC_GEMINI_MODEL == "gemini-3.5-flash"
    assert "gemini-3.5-flash" in script
    assert "gemini-2.5-flash" not in script


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
