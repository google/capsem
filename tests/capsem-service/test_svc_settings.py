"""Settings endpoints: /settings, /settings/presets, /settings/presets/{id},
/settings/lint, /settings/validate-key.

These endpoints read and write under CAPSEM_HOME (user.toml, corp.toml).
The conftest's `service_env` fixture isolates CAPSEM_HOME to a tmpdir,
so mutations here never touch the developer's real ~/.capsem/.
"""

import pytest

pytestmark = pytest.mark.integration


class TestSettingsTree:

    def test_settings_response_shape(self, client):
        """/settings returns tree + issues + presets bundled for the frontend."""
        resp = client.get("/settings")
        assert resp is not None
        for key in ("tree", "issues", "presets"):
            assert key in resp, f"missing '{key}': {list(resp.keys())}"
        assert isinstance(resp["tree"], list) and resp["tree"], "empty tree"
        assert isinstance(resp["issues"], list)
        assert isinstance(resp["presets"], list) and resp["presets"], "empty presets"

    def test_save_settings_round_trips(self, client):
        """POST /settings toggles a bool and GET reflects the new value.

        `app.auto_update` is a baseline bool (default: true). Flipping it
        to false and re-reading proves write-through works against the
        isolated CAPSEM_HOME user.toml. Leaves it flipped -- teardown drops
        the tmpdir with the rest of the isolated home.
        """
        before = _find_setting_value(client.get("/settings")["tree"], "app.auto_update")
        assert before is True, f"default expected true, got {before}"

        saved = client.post("/settings", {"app.auto_update": False})
        assert saved is not None, "POST /settings returned no body"
        # Response mirrors GET: tree + issues + presets.
        assert "tree" in saved and "issues" in saved and "presets" in saved

        after = _find_setting_value(saved["tree"], "app.auto_update")
        assert after is False, f"save did not apply: {after}"

        # Fresh GET confirms persistence.
        refetched = _find_setting_value(client.get("/settings")["tree"], "app.auto_update")
        assert refetched is False

    def test_save_settings_rejects_unknown_key(self, client):
        """Batch update is atomic -- any unknown key fails the whole batch."""
        resp = client.post("/settings", {"totally.not.a.setting": True})
        # UdsHttpClient returns whatever the body contains on error; the
        # contract is that the batch was rejected.
        assert resp is None or "error" in resp or "unknown" in str(resp).lower(), (
            f"unknown key should reject batch: {resp}"
        )


class TestPresets:

    def test_presets_lists_medium_and_high(self, client):
        """/settings/presets returns the compile-time embedded presets."""
        resp = client.get("/settings/presets")
        assert isinstance(resp, list) and resp, f"presets empty: {resp}"
        ids = {p["id"] for p in resp}
        assert {"medium", "high"}.issubset(ids), f"expected medium+high, got {ids}"
        for preset in resp:
            for key in ("id", "name", "description", "settings"):
                assert key in preset, f"preset missing '{key}': {preset}"

    def test_apply_preset_returns_refreshed_tree(self, client):
        """POST /settings/presets/{id} applies settings and returns the new tree."""
        resp = client.post("/settings/presets/high", {})
        assert resp is not None
        # apply_preset returns the same shape as GET /settings.
        for key in ("tree", "issues", "presets"):
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


def _find_setting_value(tree, dotted_id):
    """Return the leaf's effective_value for a setting addressed by dotted id.

    SettingsNode is a tagged enum: groups carry `children`; leaves carry the
    flattened ResolvedSetting fields including `id` (full dotted path) and
    `effective_value`. Actions/mcp_server nodes have neither.
    """

    def walk(node):
        kind = node.get("kind")
        if kind == "leaf" and node.get("id") == dotted_id:
            return node.get("effective_value")
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
