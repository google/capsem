"""Settings endpoints: /settings, /settings/presets, /settings/presets/{id},
/settings/lint, /settings/validate-key.

These endpoints read and write Profile V2 state under CAPSEM_HOME.
The conftest's `service_env` fixture isolates CAPSEM_HOME to a tmpdir,
so mutations here never touch the developer's real ~/.capsem/.
"""

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.integration


@pytest.fixture
def isolated_client():
    """One-off service whose CAPSEM_HOME is not shared with other tests.

    The session-scoped `service_env` is reused by every test in the
    `tests/capsem-service/` worker. Preset application writes keys like
    Profile V2 settings into that shared CAPSEM_HOME,
    which then leaks into `test_svc_mcp_api.py::test_policy_returns_merged_shape`
    (which expects the unset-default `"allow"`). Any test that mutates
    service/profile state other tests depend on should use this fixture instead.
    """
    svc = ServiceInstance()
    svc.start()
    try:
        yield svc.client()
    finally:
        svc.stop()


class TestSettingsProfiles:

    def test_settings_response_shape(self, client):
        """/settings returns typed settings-profiles payload only."""
        resp = client.get("/settings")
        assert resp is not None
        for key in ("settings_profiles", "profile_presets", "effective_rules", "mode"):
            assert key in resp, f"missing '{key}': {list(resp.keys())}"
        assert isinstance(resp["settings_profiles"], dict)
        assert isinstance(resp["profile_presets"], list) and resp["profile_presets"], "empty profile_presets"
        assert isinstance(resp["effective_rules"], dict)
        assert resp["mode"] == "settings_profiles_v2"

    def test_save_settings_round_trips(self, isolated_client):
        """POST /settings writes policy rules and GET reflects persisted rule state."""
        client = isolated_client
        rule_key = "policy.http.block_example_org"
        rule = {
            "on": "http.request",
            "if": "request.host == 'example.org'",
            "decision": "block",
            "priority": 1,
            "reason": "integration test rule",
        }
        saved = client.post("/settings", {rule_key: rule})
        assert saved is not None, "POST /settings returned no body"
        assert "effective_rules" in saved and "settings_profiles" in saved

        after = saved["effective_rules"]["http"]["block_example_org"]
        assert after["decision"] == "block", f"save did not apply: {after}"

        refetched = client.get("/settings")["effective_rules"]["http"]["block_example_org"]
        assert refetched["decision"] == "block"

    def test_save_settings_rejects_unknown_key(self, client):
        """Batch update is atomic -- any unknown key fails the whole batch."""
        resp = client.post("/settings", {"totally.not.a.setting": True})
        # UdsHttpClient returns whatever the body contains on error; the
        # contract is that the batch was rejected.
        assert resp is None or "error" in resp or "unknown" in str(resp).lower(), (
            f"unknown key should reject batch: {resp}"
        )


class TestPresets:

    def test_presets_lists_profile_ids(self, client):
        """/settings/presets returns available profile presets from profile roots."""
        resp = client.get("/settings/presets")
        assert isinstance(resp, list) and resp, f"presets empty: {resp}"
        ids = {p["id"] for p in resp}
        assert "everyday-work" in ids, f"expected everyday-work profile preset, got {ids}"
        for preset in resp:
            for key in ("id", "name", "description", "settings"):
                assert key in preset, f"preset missing '{key}': {preset}"

    def test_select_profile_preset_returns_refreshed_tree(self, isolated_client):
        """POST /settings/presets/{id} selects a profile and returns typed payload.

        Uses `isolated_client` because selecting a preset mutates shared
        CAPSEM_HOME state (default profile selection) that
        leaks into sibling files' assertions about the unset default.
        """
        resp = isolated_client.post("/settings/presets/everyday-work", {})
        assert resp is not None
        for key in ("settings_profiles", "profile_presets", "effective_rules", "mode"):
            assert key in resp, f"missing '{key}': {list(resp.keys())}"

    def test_apply_unknown_preset_rejected(self, client):
        """Unknown preset IDs must fail with a 400-class error."""
        resp = client.post("/settings/presets/doesnotexist", {})
        assert resp is None or "error" in resp or "unknown" in str(resp).lower(), (
            f"unknown preset should reject: {resp}"
        )


class TestLint:

    def test_lint_returns_array(self, client):
        """POST /settings/lint returns the issues array (possibly empty)."""
        resp = client.post("/settings/lint", {})
        assert isinstance(resp, list), f"lint did not return list: {resp!r}"


class TestValidateKey:

    def test_validate_key_unknown_provider_rejected(self, client):
        """Unknown provider must 400; don't issue a network call."""
        resp = client.post("/settings/validate-key", {
            "provider": "not-a-real-provider",
            "key": "whatever",
        })
        assert resp is None or "error" in resp or "unknown" in str(resp).lower(), (
            f"unknown provider should reject: {resp}"
        )

    def test_validate_key_empty_key_not_valid(self, client):
        """Empty key short-circuits before the network call and reports invalid."""
        resp = client.post("/settings/validate-key", {
            "provider": "anthropic",
            "key": "",
        })
        assert resp is not None, "validate-key returned no body"
        assert resp.get("valid") is False, f"expected valid=false for empty key: {resp}"
        assert isinstance(resp.get("message"), str) and resp["message"], (
            f"missing message: {resp}"
        )

    def test_validate_key_bogus_anthropic_returns_invalid(self, client):
        """A syntactically-plausible-but-wrong key returns valid=false via real HTTP.

        This makes a live call to api.anthropic.com. If there's no network
        (CI, air-gapped), the handler still returns a KeyValidation with
        valid=false and a "Connection failed"/"Network error" message --
        so the shape assertion holds either way.
        """
        resp = client.post(
            "/settings/validate-key",
            {"provider": "anthropic", "key": "sk-ant-not-a-real-key-xyz"},
            timeout=30,
        )
        assert resp is not None, "validate-key returned no body"
        assert resp.get("valid") is False, f"bogus key reported valid: {resp}"
        assert isinstance(resp.get("message"), str) and resp["message"], (
            f"missing message: {resp}"
        )
