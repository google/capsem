"""First-class install-adjacent service endpoints.

The setup/onboarding API is intentionally gone. Assets and corporate policy
configuration now have direct routes that do not depend on setup state.
The conftest's `service_env` fixture isolates CAPSEM_HOME so mutations here
never touch the developer's real config.
"""

import re
from pathlib import Path

import pytest

pytestmark = pytest.mark.integration


class TestSetupRoutesRemoved:

    def test_setup_state_route_is_removed(self, client):
        assert client.get("/setup/state") is None

    def test_setup_complete_route_is_removed(self, client):
        assert client.post("/setup/complete", {}) is None

    def test_setup_assets_alias_is_removed(self, client):
        assert client.get("/setup/assets") is None

    def test_setup_corp_config_alias_is_removed(self, client):
        assert client.post("/setup/corp-config", {}) is None

    def test_retired_corp_config_route_is_removed(self, client):
        assert client.post("/corp-config", {}) is None


class TestAssets:

    def test_assets_lists_three_expected_artifacts(self, client):
        """GET /assets/status enumerates vmlinuz, initrd.img, and rootfs."""
        resp = client.get("/assets/status")
        assert resp is not None
        # Handler either returns {ready, downloading, asset_version, assets}
        # or {ready: false, downloading: false, error, assets: []}.
        assert "ready" in resp and "assets" in resp, f"missing keys: {resp}"
        assert isinstance(resp["ready"], bool)
        assert isinstance(resp["assets"], list)
        if resp["assets"]:
            names = {a["name"] for a in resp["assets"]}
            assert "vmlinuz" in names
            assert "initrd.img" in names
            rootfs_names = names - {"vmlinuz", "initrd.img"}
            assert len(rootfs_names) == 1, f"unexpected asset names: {names}"
            rootfs_name = next(iter(rootfs_names))
            assert rootfs_name == "rootfs.squashfs" or re.fullmatch(
                r"rootfs(?:-[a-f0-9]{16})?\.erofs",
                rootfs_name,
            ), f"unexpected rootfs asset name: {rootfs_name}"
            for asset in resp["assets"]:
                assert asset["status"] in ("present", "missing")

    def test_assets_reports_ready_when_all_present(self, client):
        """ready=true iff every listed asset has status=present.

        Test binaries are spawned with --assets-dir pointing at the real
        repo assets, so in a dev environment this should be ready=true.
        If assets haven't been built yet, we accept ready=false but still
        verify the invariant.
        """
        resp = client.get("/assets/status")
        assert resp is not None
        if resp.get("error"):
            # No asset manifest -- skip the invariant but keep shape assertion.
            return
        all_present = all(a["status"] == "present" for a in resp["assets"])
        assert resp["ready"] == all_present, (
            f"ready={resp['ready']} but all_present={all_present}: {resp}"
        )

    def test_assets_ensure_returns_status_shape(self, client):
        """POST /assets/ensure returns the same status shape after reconcile."""
        resp = client.post("/assets/ensure", {})
        assert resp is not None
        assert "ready" in resp and "assets" in resp, f"missing keys: {resp}"
        assert resp.get("ensured") is True or resp.get("error") is not None
        assert isinstance(resp["ready"], bool)
        assert isinstance(resp["assets"], list)


class TestCorpConfig:

    def test_corp_edit_inline_toml(self, client):
        """PUT /corp/edit with inline TOML writes corp.toml.

        Validates against policy_config::corp_provision::install_inline_corp_config.
        Empty [settings] is a valid corp config that locks no settings.
        """
        toml_content = (
            "refresh_interval_hours = 24\n"
            "\n"
            "[settings]\n"
            '"ai.openai.allow" = { value = false, modified = "2026-04-21T00:00:00Z" }\n'
        )
        resp = client.put("/corp/edit", {"toml": toml_content})
        assert resp is not None and resp.get("success") is True, (
            f"corp edit inline failed: {resp}"
        )

        # Corp-locked setting must now appear as corp_locked in the tree.
        tree = client.get("/settings")["tree"]
        locked = _find_setting_flag(tree, "ai.openai.allow", "corp_locked")
        assert locked is True, f"corp-locked not surfaced after install: {locked}"

    def test_corp_config_rejects_invalid_toml(self, client):
        """Malformed TOML must be rejected with a 400-class error."""
        resp = client.put("/corp/edit", {"toml": "this is [ broken"})
        assert resp is None or "error" in resp or "invalid" in str(resp).lower(), (
            f"invalid corp TOML should reject: {resp}"
        )

    def test_corp_config_rejects_empty_payload(self, client):
        """Body with neither `source` nor `toml` must be rejected."""
        resp = client.put("/corp/edit", {})
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
