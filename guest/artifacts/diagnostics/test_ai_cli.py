"""AI CLI installation and sandbox enforcement tests."""

import json
import os
import re
import tomllib
from urllib.parse import urlsplit

import pytest

from conftest import run

LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"
SECRET_PATTERN = re.compile(
    r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{20,})"
)


def _require_local_mock_url(path, reason):
    base_url = os.environ.get(LOCAL_MOCK_SERVER_ENV)
    if not base_url:
        pytest.fail(f"{reason}; set {LOCAL_MOCK_SERVER_ENV}")
    url = f"{base_url.rstrip('/')}/{path.lstrip('/')}"
    parsed = urlsplit(url)
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    if parsed.scheme == "http" and port not in (80, 3128, 3713, 8080, 11434):
        pytest.fail(
            f"{reason}; local mock server port {port} is outside the "
            "default HTTP upstream allowlist"
        )
    return url


@pytest.mark.parametrize("cli", ["claude", "gemini", "codex"])
def test_ai_cli_installed(cli):
    """AI CLI binary must be in PATH."""
    result = run(f"command -v {cli}")
    assert result.returncode == 0, f"{cli} not found in PATH"


def test_opt_ai_clis_bin_in_path():
    """/opt/ai-clis/bin must be in PATH (npm global prefix)."""
    result = run("echo $PATH")
    assert "/opt/ai-clis/bin" in result.stdout, (
        f"/opt/ai-clis/bin not in PATH: {result.stdout}"
    )


def test_no_npm_global_in_path():
    """Old .npm-global path must NOT be in PATH."""
    result = run("echo $PATH")
    assert ".npm-global" not in result.stdout, (
        f"stale .npm-global still in PATH: {result.stdout}"
    )


def test_codex_sandbox_prerequisite_bubblewrap_available():
    """Codex should not fall back to bundled sandbox helpers because bwrap is missing."""
    result = run("command -v bwrap && bwrap --version", timeout=10)
    assert result.returncode == 0, (
        f"bubblewrap/bwrap missing from PATH: {result.stdout} {result.stderr}"
    )
    assert "bubblewrap" in result.stdout.lower()


def test_npm_prefix_is_opt_ai_clis():
    """npm global prefix must point to /opt/ai-clis."""
    # A four-VM qualification run can make Node startup exceed the generic
    # 10-second command budget even though npm is healthy. Keep this explicit
    # and bounded so the prerequisite still fails fast with load diagnostics.
    result = run("npm config get prefix", timeout=30)
    assert result.returncode == 0, f"npm config get prefix failed: {result.stderr}"
    assert "/opt/ai-clis" in result.stdout.strip(), (
        f"npm prefix wrong: {result.stdout.strip()}"
    )


@pytest.mark.parametrize("cli", ["claude", "gemini", "codex"])
def test_ai_cli_in_login_shell(cli):
    """AI CLI must be findable from a login shell (what the user actually sees)."""
    result = run(f"bash -lc 'which {cli}'", timeout=10)
    assert result.returncode == 0, (
        f"{cli} not found in login shell PATH: {result.stdout} {result.stderr}"
    )


@pytest.mark.parametrize("cli", ["claude", "agy"])
def test_user_local_ai_cli_shim_survives_runtime_root_mount(cli):
    """Curl-installed CLIs must keep the user-local shim their doctors expect."""
    result = run(f"readlink -f /root/.local/bin/{cli}", timeout=10)
    assert result.returncode == 0, (
        f"/root/.local/bin/{cli} shim missing: {result.stdout} {result.stderr}"
    )
    assert result.stdout.strip() == f"/usr/local/bin/{cli}"


@pytest.mark.parametrize("cli", ["gemini", "claude", "codex"])
def test_ai_cli_help(cli):
    """AI CLI --help must execute without runtime errors."""
    result = run(f"{cli} --help 2>&1", timeout=15)
    output = result.stdout
    # Must not crash with a JS/Node runtime error
    for error in ["SyntaxError", "TypeError", "ReferenceError", "Cannot find module"]:
        assert error not in output, f"{cli} --help has runtime error: {error}"
    assert result.returncode == 0, (
        f"{cli} --help failed (rc={result.returncode}): {output[:300]}"
    )


# ---------------------------------------------------------------
# Google AI / Gemini configuration
# ---------------------------------------------------------------


def test_gemini_api_key_no_duplicate():
    """GOOGLE_API_KEY must NOT be set alongside GEMINI_API_KEY (gemini CLI warns)."""
    google = os.environ.get("GOOGLE_API_KEY")
    if google and os.environ.get("GEMINI_API_KEY"):
        pytest.fail(
            "Both GOOGLE_API_KEY and GEMINI_API_KEY are set -- "
            "gemini CLI will warn. Only GEMINI_API_KEY should be injected."
        )


def _read_json(path):
    result = run(f"cat {path}")
    assert result.returncode == 0, f"missing profile-owned JSON {path}: {result.stderr}"
    assert not SECRET_PATTERN.search(result.stdout), f"secret-like value found in {path}"
    return json.loads(result.stdout)


def test_gemini_profile_config_seeded_without_credentials():
    """Profile-owned Gemini config must be projected at boot without secrets."""
    settings = _read_json("/root/.gemini/settings.json")
    assert settings["general"]["enableAutoUpdate"] is False
    assert settings["general"]["enableAutoUpdateNotification"] is False
    assert settings["privacy"]["usageStatisticsEnabled"] is False
    assert settings["privacy"]["sessionRetention"] == "none"
    assert settings["telemetry"]["enabled"] is False
    assert settings["security"]["auth"]["selectedType"] == "gemini-api-key"
    assert settings["security"]["folderTrust.enabled"] is False

    projects = _read_json("/root/.gemini/projects.json")
    assert projects["projects"]["/root"] == "root"

    trusted = _read_json("/root/.gemini/trustedFolders.json")
    assert trusted["/root"] == "TRUST_FOLDER"

    installation_id = run("cat /root/.gemini/installation_id")
    assert installation_id.returncode == 0
    assert installation_id.stdout.strip()
    assert not SECRET_PATTERN.search(installation_id.stdout)


def test_antigravity_profile_config_seeded_without_credentials():
    """Profile-owned Antigravity config must be projected at boot without secrets."""
    settings = _read_json("/root/.antigravity/settings.json")
    assert settings["colorScheme"] == "dark"
    assert "/root" in settings["trustedWorkspaces"]
    assert not SECRET_PATTERN.search(json.dumps(settings, sort_keys=True))


def test_codex_profile_config_uses_first_party_defaults_without_local_ollama():
    """Default Codex profile config must not force a local Ollama provider."""
    result = run("cat /root/.codex/config.toml")
    assert result.returncode == 0, f"missing Codex profile config: {result.stderr}"
    assert not SECRET_PATTERN.search(result.stdout), "secret-like value found in Codex config"
    config = tomllib.loads(result.stdout)

    assert config.get("model_provider") not in {"local_ollama", "ollama"}, (
        "Codex default profile must not be preconfigured to Ollama; local providers "
        "belong behind explicit profile/rule/user selection."
    )
    providers = config.get("model_providers") or {}
    assert "local_ollama" not in providers
    assert "ollama" not in providers


def test_google_ai_local_fixture_allowed():
    """Google AI-shaped local fixture must be reachable through the MITM proxy."""
    local_url = _require_local_mock_url("/tiny", "local Google AI fixture smoke")
    result = run(
        f"curl -sI --connect-timeout 10 {local_url} 2>&1",
        timeout=20,
    )
    assert result.returncode == 0, (
        f"local Google AI fixture should be allowed: {result.stdout}\n{result.stderr}"
    )
    assert "HTTP/" in result.stdout, (
        f"no HTTP response from local Google AI fixture: {result.stdout}"
    )
