"""Synthetic auth adapter source proofs; never a real CLI/native turn."""

import json
import shlex
import subprocess
import sys

import pytest
from helpers.agy_auth import synthetic_home_setup
from helpers.bounded import bounded
from ironbank.model_client_config import HERMETIC_AGY_MODEL_DISPLAY

from tests.qualification import agent_credentials, candidate


def test_shared_agy_setup_keeps_existing_format_and_shipped_mcp(tmp_path):
    home = tmp_path / "home"
    mcp = home / ".gemini/config/mcp_config.json"
    mcp.parent.mkdir(parents=True)
    mcp.write_text("shipped-mcp-config")
    script = (
        "import json\nfrom pathlib import Path\n"
        f"HOME={json.dumps(str(home))}\nWORKSPACE='/workspace'\nNONCE={'ab' * 16!r}\n"
        + synthetic_home_setup()
    )
    subprocess.run(bounded([sys.executable, "-c", script], 10), check=True)
    token = home / ".gemini/antigravity-cli/antigravity-oauth-token"
    assert token.stat().st_mode & 0o777 == 0o600
    assert json.loads(token.read_text()) == {
        "token": {"access_token": "capsem_test_agy_access_" + "ab" * 16,
                  "token_type": "Bearer", "refresh_token": "capsem_test_agy_refresh_" + "ab" * 16,
                  "expiry": "2099-01-01T00:00:00Z"}, "auth_method": "consumer",
    }
    settings = json.loads((token.parent / "settings.json").read_text())
    assert settings == {"trustedWorkspaces": ["/workspace"], "telemetry": {"enabled": False}, "autoUpdate": {"enabled": False}}
    assert mcp.read_text() == "shipped-mcp-config"


def test_explicit_existing_oauth_profile_does_not_invent_api_key(tmp_path, monkeypatch):
    image = tmp_path / "images" / "agy"
    image.mkdir(parents=True)
    declaration = '''capabilities = ["agent"]
[expect]
turn = ["agy", "-p"]
credential_fixture = "agy_oauth"
provider = "google"
path = "/v1internal:streamGenerateContent"
'''
    path = image / "qualify.toml"
    path.write_text(declaration)
    monkeypatch.setattr(candidate, "ROOT", tmp_path)
    assert candidate.declared_capabilities("agy") == {"agent"}
    for invalid in (
        declaration.replace("agy_oauth", "unknown"),
        declaration.replace('provider = "google"', 'provider = "openai"'),
        declaration.replace('turn = ["agy", "-p"]', 'turn = ["other", "-p"]'),
        declaration.replace('/v1internal:streamGenerateContent', '/wrong'),
        declaration.replace('credential_fixture = "agy_oauth"', 'credential_fixture = "agy_oauth"\ncredential_env = "GEMINI_API_KEY"'),
    ):
        path.write_text(invalid)
        with pytest.raises(ValueError, match="credential"):
            candidate.declared("agy")


def test_shipped_agy_declaration_matches_existing_model_and_prompt_argument_contract():
    expect = candidate.declared("agy")["expect"]
    assert expect["credential_fixture"] == "agy_oauth"
    assert "credential_env" not in expect
    assert expect["turn"] == ["agy", "--model", HERMETIC_AGY_MODEL_DISPLAY, "--print-timeout", "90s", "-p"]


def test_qualification_prepares_original_workload_home_only(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setenv("HOME", str(tmp_path))

    def workload(client, vm_id, command):
        calls.append((client, vm_id))
        argv = shlex.split(command)
        assert argv[:2] == ["python3", "-c"]
        subprocess.run(bounded([sys.executable, "-c", argv[2]], 10), check=True)
        return {"exit_code": 0}

    monkeypatch.setattr(agent_credentials, "workload_exec", workload)
    oauth = {"credential_fixture": "agy_oauth"}
    assert agent_credentials.environment(oauth) == {}
    assert agent_credentials.command_environment(oauth) == ""
    agent_credentials.prepare("owned-client", "owned-vm", oauth)
    assert calls == [("owned-client", "owned-vm")]
    token = json.loads((tmp_path / ".gemini/antigravity-cli/antigravity-oauth-token").read_text())
    assert token["auth_method"] == "consumer"
    assert len(token["token"]["access_token"].removeprefix("capsem_test_agy_access_")) == 32
    key = {"credential_env": "OPENAI_API_KEY"}
    assert agent_credentials.environment(key) == {"OPENAI_API_KEY": "capsem-qualification-test-key"}
    agent_credentials.prepare("unused", "unused", key)
    assert len(calls) == 1
    monkeypatch.setattr(agent_credentials, "workload_exec", lambda *args: {"exit_code": 9})
    with pytest.raises(AssertionError):
        agent_credentials.prepare("owned-client", "owned-vm", oauth)
