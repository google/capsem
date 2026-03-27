"""AI CLI installation and sandbox enforcement tests."""

import os

import pytest

from conftest import run


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


def test_npm_prefix_is_opt_ai_clis():
    """npm global prefix must point to /opt/ai-clis."""
    result = run("npm config get prefix")
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


def test_gemini_settings_exist():
    """Gemini CLI settings.json must be seeded with valid config."""
    result = run("cat /root/.gemini/settings.json 2>&1")
    assert result.returncode == 0, "~/.gemini/settings.json missing"
    assert "homeDirectoryWarningDismissed" in result.stdout
    assert "sessionRetention" in result.stdout
    assert "gemini-api-key" in result.stdout


def test_gemini_projects_exist():
    """Gemini CLI projects.json must register /root as a project."""
    result = run("cat /root/.gemini/projects.json 2>&1")
    assert result.returncode == 0, "~/.gemini/projects.json missing"
    assert "/root" in result.stdout


def test_gemini_trusted_folders_exist():
    """Gemini CLI trustedFolders.json must trust /root."""
    result = run("cat /root/.gemini/trustedFolders.json 2>&1")
    assert result.returncode == 0, "~/.gemini/trustedFolders.json missing"
    assert "TRUST_FOLDER" in result.stdout


def test_gemini_installation_id_exist():
    """Gemini CLI installation_id must be present."""
    result = run("cat /root/.gemini/installation_id 2>&1")
    assert result.returncode == 0, "~/.gemini/installation_id missing"
    assert len(result.stdout.strip()) > 0, "installation_id is empty"


def test_google_ai_domain_allowed():
    """Google AI domain must be reachable through the MITM proxy."""
    result = run(
        "curl -sI --connect-timeout 10 https://generativelanguage.googleapis.com 2>&1",
        timeout=20,
    )
    # TLS handshake should succeed, HTTP response received (even if 404/401)
    assert result.returncode == 0, (
        f"Google AI domain should be allowed: {result.stdout}\n{result.stderr}"
    )
    assert "HTTP/" in result.stdout, (
        f"no HTTP response from Google AI domain: {result.stdout}"
    )
