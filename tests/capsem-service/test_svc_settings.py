"""Settings endpoints: /settings/info and /settings/edit.

These endpoints read and write under CAPSEM_HOME (settings.toml, corp.toml).
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
    user settings into that shared CAPSEM_HOME,
    which then leaks into `test_svc_mcp_api.py::test_policy_returns_merged_shape`
    (which expects the unset-default `"allow"`). Any test that mutates
    settings.toml state other tests depend on should use this fixture instead.
    """
    svc = ServiceInstance()
    svc.start()
    try:
        yield svc.client()
    finally:
        svc.stop()


class TestSettingsTree:

    def test_settings_response_shape(self, client):
        """/settings/info returns UI/app settings data without behavior presets."""
        resp = client.get("/settings/info")
        assert resp is not None
        for key in ("tree", "issues"):
            assert key in resp, f"missing '{key}': {list(resp.keys())}"
        assert "presets" not in resp, f"settings response leaked presets: {resp.keys()}"
        assert isinstance(resp["tree"], list) and resp["tree"], "empty tree"
        assert isinstance(resp["issues"], list)

    def test_save_settings_round_trips(self, client):
        """PATCH /settings/edit toggles a bool and GET reflects the new value.

        `app.auto_update` is a baseline bool (default: true). Flipping it
        to false and re-reading proves write-through works against the
        isolated CAPSEM_HOME settings.toml. Leaves it flipped -- teardown drops
        the tmpdir with the rest of the isolated home.
        """
        before = _find_setting_value(client.get("/settings/info")["tree"], "app.auto_update")
        assert before is True, f"default expected true, got {before}"

        saved = client.patch("/settings/edit", {"app.auto_update": False})
        assert saved is not None, "PATCH /settings/edit returned no body"
        # Response mirrors GET: tree + issues, without behavior presets.
        assert "tree" in saved and "issues" in saved and "presets" not in saved

        after = _find_setting_value(saved["tree"], "app.auto_update")
        assert after is False, f"save did not apply: {after}"

        # Fresh GET confirms persistence.
        refetched = _find_setting_value(client.get("/settings/info")["tree"], "app.auto_update")
        assert refetched is False

    def test_save_settings_rejects_unknown_key(self, client):
        """Batch update is atomic -- any unknown key fails the whole batch."""
        resp = client.patch("/settings/edit", {"totally.not.a.setting": True})
        # UdsHttpClient returns whatever the body contains on error; the
        # contract is that the batch was rejected.
        assert resp is None or "error" in resp or "unknown" in str(resp).lower(), (
            f"unknown key should reject batch: {resp}"
        )

    def test_retired_magic_settings_route_is_removed(self, client):
        """The old GET/POST /settings route must not remain as a compatibility alias."""
        assert client.get("/settings") is None
        assert client.post("/settings", {"app.auto_update": False}) is None


class TestRetiredSettingsUtilityRoutes:

    def test_presets_route_is_removed(self, client):
        assert client.get("/settings/presets") is None
        assert client.post("/settings/presets/high", {}) is None

    def test_lint_route_is_removed(self, client):
        assert client.post("/settings/lint", {}) is None

    def test_validate_key_route_is_removed(self, client):
        assert client.post("/settings/validate-key", {
            "provider": "anthropic",
            "key": "sk-ant-not-a-real-key-xyz",
        }) is None


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
