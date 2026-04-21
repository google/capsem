"""Setup / onboarding endpoints: /setup/state, /setup/detect, /setup/complete,
/setup/assets, /setup/corp-config.

These endpoints read/write under CAPSEM_HOME (setup-state.json, corp.toml,
corp-source.json) and for /setup/detect also under HOME (~/.gitconfig,
~/.ssh, ~/.anthropic, ~/.claude, ~/.gemini, ~/.config/openai, gh auth token,
~/.config/gcloud). The conftest's `service_env` fixture isolates both,
so mutations here never touch the developer's real config.

Note: /setup/assets/download was removed in 24633a5 (dead code) so there
is no corresponding test.
"""

from pathlib import Path

import pytest

pytestmark = pytest.mark.integration


class TestSetupState:

    def test_state_defaults_when_missing(self, client):
        """GET /setup/state returns defaults when setup-state.json is missing.

        The isolated CAPSEM_HOME starts empty, so the handler walks the
        default_state_path -> load_state path that returns SetupState::default.
        """
        resp = client.get("/setup/state")
        assert resp is not None, "setup/state returned no body"
        expected = {
            "schema_version", "completed_steps", "security_preset",
            "providers_done", "repositories_done", "service_installed",
            "install_completed", "onboarding_completed", "onboarding_version",
            "needs_onboarding", "corp_config_source",
        }
        missing = expected - resp.keys()
        assert not missing, f"missing state keys: {missing}"
        # Defaults: fresh install means these booleans are false.
        assert resp["onboarding_completed"] is False
        assert resp["install_completed"] is False
        # `needs_onboarding` is computed server-side: !onboarding_completed OR
        # version<CURRENT. A fresh state must require onboarding.
        assert resp["needs_onboarding"] is True
        assert isinstance(resp["completed_steps"], list)

    def test_complete_sets_onboarding_flag(self, client):
        """POST /setup/complete flips onboarding_completed and sets version.

        After /setup/complete, a fresh /setup/state read should show
        onboarding_completed=true, onboarding_version>=1, and
        needs_onboarding=false.
        """
        resp = client.post("/setup/complete", {})
        assert resp is not None and resp.get("success") is True, (
            f"complete returned unexpected body: {resp}"
        )

        after = client.get("/setup/state")
        assert after["onboarding_completed"] is True
        assert after["onboarding_version"] >= 1, f"version not set: {after}"
        assert after["needs_onboarding"] is False


class TestSetupDetect:

    def test_detect_returns_summary_shape(self, client):
        """GET /setup/detect returns DetectedConfigSummary with all presence keys.

        Only asserts shape + types, not values. API-key detection checks env
        vars before file paths (GEMINI_API_KEY, OPENAI_API_KEY,
        ANTHROPIC_API_KEY), and env is inherited from the test runner's shell
        -- any of those being set would flip presence to true regardless of
        HOME isolation. File-based presence (ssh_public_key, claude_oauth,
        google_adc) is shaped by the isolated HOME, but the correctness
        invariant we actually want to pin here is the endpoint response
        shape.
        """
        resp = client.get("/setup/detect")
        assert resp is not None
        expected = {
            "git_name", "git_email",
            "ssh_public_key_present", "anthropic_api_key_present",
            "google_api_key_present", "openai_api_key_present",
            "github_token_present", "claude_oauth_present",
            "google_adc_present", "settings_written",
        }
        missing = expected - resp.keys()
        assert not missing, f"missing detect keys: {missing}"
        # Presence keys are booleans; git_name/email are str or null.
        for key in (
            "ssh_public_key_present", "anthropic_api_key_present",
            "google_api_key_present", "openai_api_key_present",
            "github_token_present", "claude_oauth_present",
            "google_adc_present",
        ):
            assert isinstance(resp[key], bool), f"{key} not bool: {resp[key]!r}"
        assert resp["git_name"] is None or isinstance(resp["git_name"], str)
        assert resp["git_email"] is None or isinstance(resp["git_email"], str)
        assert isinstance(resp["settings_written"], list)
        # File-based detections read HOME, which is isolated to a fresh
        # tmpdir, so these must be false regardless of env-var credentials.
        assert resp["ssh_public_key_present"] is False, (
            "ssh key detected in isolated HOME -- fixture leaked"
        )
        assert resp["claude_oauth_present"] is False
        assert resp["google_adc_present"] is False


class TestSetupAssets:

    def test_assets_lists_three_expected_artifacts(self, client):
        """GET /setup/assets enumerates vmlinuz, initrd.img, rootfs.squashfs."""
        resp = client.get("/setup/assets")
        assert resp is not None
        # Handler either returns {ready, downloading, asset_version, assets}
        # or {ready: false, downloading: false, error, assets: []}.
        assert "ready" in resp and "assets" in resp, f"missing keys: {resp}"
        assert isinstance(resp["ready"], bool)
        assert isinstance(resp["assets"], list)
        if resp["assets"]:
            names = {a["name"] for a in resp["assets"]}
            assert names == {"vmlinuz", "initrd.img", "rootfs.squashfs"}, (
                f"unexpected asset names: {names}"
            )
            for asset in resp["assets"]:
                assert asset["status"] in ("present", "missing")

    def test_assets_reports_ready_when_all_present(self, client):
        """ready=true iff every listed asset has status=present.

        Test binaries are spawned with --assets-dir pointing at the real
        repo assets, so in a dev environment this should be ready=true.
        If assets haven't been built yet, we accept ready=false but still
        verify the invariant.
        """
        resp = client.get("/setup/assets")
        assert resp is not None
        if resp.get("error"):
            # No asset manifest -- skip the invariant but keep shape assertion.
            return
        all_present = all(a["status"] == "present" for a in resp["assets"])
        assert resp["ready"] == all_present, (
            f"ready={resp['ready']} but all_present={all_present}: {resp}"
        )


class TestSetupCorpConfig:

    def test_corp_config_inline_toml(self, client):
        """POST /setup/corp-config with inline TOML writes corp.toml.

        Validates against policy_config::corp_provision::install_inline_corp_config.
        Empty [settings] is a valid corp config that locks no settings.
        """
        toml_content = (
            "refresh_interval_hours = 24\n"
            "\n"
            "[settings]\n"
            '"ai.openai.allow" = { value = false, modified = "2026-04-21T00:00:00Z" }\n'
        )
        resp = client.post("/setup/corp-config", {"toml": toml_content})
        assert resp is not None and resp.get("success") is True, (
            f"corp-config inline failed: {resp}"
        )

        # Corp-locked setting must now appear as corp_locked in the tree.
        tree = client.get("/settings")["tree"]
        locked = _find_setting_flag(tree, "ai.openai.allow", "corp_locked")
        assert locked is True, f"corp-locked not surfaced after install: {locked}"

    def test_corp_config_rejects_invalid_toml(self, client):
        """Malformed TOML must be rejected with a 400-class error."""
        resp = client.post("/setup/corp-config", {"toml": "this is [ broken"})
        assert resp is None or "error" in resp or "invalid" in str(resp).lower(), (
            f"invalid corp TOML should reject: {resp}"
        )

    def test_corp_config_rejects_empty_payload(self, client):
        """Body with neither `source` nor `toml` must be rejected."""
        resp = client.post("/setup/corp-config", {})
        assert resp is None or "error" in resp or "provide either" in str(resp).lower(), (
            f"empty payload should reject: {resp}"
        )


def _find_setting_flag(tree, dotted_id, flag):
    """Walk the tree for a leaf matching dotted_id and return `flag` on the leaf."""

    def walk(node):
        if node.get("kind") == "leaf" and node.get("id") == dotted_id:
            return node.get(flag)
        for child in node.get("children") or []:
            found = walk(child)
            if found is not None:
                return found
        return None

    for root in tree:
        found = walk(root)
        if found is not None:
            return found
    return None
