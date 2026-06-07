"""S08 Profile V2 gateway surface contract tests."""

import uuid
import json

import pytest

pytestmark = pytest.mark.gateway


def test_profile_catalog_and_revision_statuses_proxy_exactly(gw_client):
    catalog = gw_client.get("/profiles/catalog")

    assert catalog["mode"] == "settings_profiles_v2"
    revisions = catalog["profiles"][0]["revisions"]
    assert {item["status"] for item in revisions} == {
        "active",
        "deprecated",
        "revoked",
    }
    assert "removed" not in {item["status"] for item in revisions}

    detail = gw_client.get("/profiles/everyday-work/revisions")
    assert detail["profile_id"] == "everyday-work"
    assert {item["status"] for item in detail["revisions"]} == {
        "active",
        "deprecated",
        "revoked",
    }


def test_profile_revision_lifecycle_routes_proxy_typed_summaries(gw_client):
    reconcile = gw_client.post(
        "/profiles/catalog/reconcile",
        {"manifest_json": "{}", "profile_payload_pubkey": "test"},
    )
    assert reconcile["summary"]["installed"] == 1
    assert reconcile["outcomes"][0]["outcome"] == "installed"

    installed = gw_client.post("/profiles/everyday-work/revisions/install", {})
    assert installed["action"] == "install"
    assert installed["selected_revision"] == "2026.0520.1"
    assert installed["outcome"]["outcome"] == "installed"

    updated = gw_client.post("/profiles/everyday-work/revisions/update", {})
    assert updated["action"] == "update"
    assert updated["outcome"]["outcome"] == "unchanged"

    removed = gw_client.post("/profiles/everyday-work/revisions/remove", {})
    assert removed["action"] == "remove"
    assert removed["outcome"]["outcome"] == "removed"


def test_profile_crud_and_resolve_routes_proxy_profile_v2_envelope(gw_client):
    created = gw_client.post(
        "/profiles",
        {
            "version": 1,
            "id": "gateway-profile",
            "name": "Gateway Profile",
            "best_for": "Gateway tests",
            "profile_type": "custom",
        },
    )
    assert created["mode"] == "settings_profiles_v2"
    assert created["profile"]["id"] == "gateway-profile"

    listed = gw_client.get("/profiles")
    assert listed["mode"] == "settings_profiles_v2"
    assert listed["profiles"][0]["id"] == "everyday-work"

    fetched = gw_client.get("/profiles/everyday-work")
    assert fetched["profile"]["id"] == "everyday-work"

    forked = gw_client.post(
        "/profiles/everyday-work/fork",
        {"id": "gateway-fork", "name": "Gateway Fork"},
    )
    assert forked["profile"]["id"] == "gateway-fork"

    updated = gw_client.put(
        "/profiles/everyday-work",
        {
            "version": 1,
            "id": "everyday-work",
            "name": "Everyday Work Updated",
            "best_for": "Gateway tests",
            "profile_type": "custom",
        },
    )
    assert updated["profile"]["name"] == "Everyday Work Updated"

    effective = gw_client.get("/profiles/everyday-work/effective")
    assert effective["profile_id"] == "everyday-work"
    assert effective["effective"]["profile_id"] == "everyday-work"


def test_skills_mcp_rules_and_confirm_proxy_profile_v2_routes(gw_client):
    suffix = uuid.uuid4().hex[:8]
    skill_id = f"gateway-skill-{suffix}"
    connector_id = f"gateway-mcp-{suffix}"
    rule_id = "security.rules.http.ask_probe"

    skill = gw_client.post("/skills", {"id": skill_id, "kind": "enabled"})
    assert skill["id"] == skill_id
    assert skill["kind"] == "enabled"
    listed_skills = gw_client.get("/skills?kind=enabled")
    assert skill_id in listed_skills["enabled"]
    deleted_skill = gw_client.delete(f"/skills/{skill_id}?kind=enabled")
    assert deleted_skill["removed"] is True

    connector = gw_client.post(
        "/mcp/connectors",
        {
            "id": connector_id,
            "server": {"command": "npx", "args": ["@capsem/mock"]},
        },
    )
    assert connector["id"] == connector_id
    listed_connectors = gw_client.get("/mcp/connectors")
    assert any(item["id"] == connector_id for item in listed_connectors["servers"])
    deleted_connector = gw_client.delete(f"/mcp/connectors/{connector_id}")
    assert deleted_connector["removed"] is True

    rule = gw_client.post(
        "/rules",
        {
            "id": rule_id,
            "callback": "http.request",
            "condition": "request.host == 'probe.example.com'",
            "decision": "ask",
            "priority": 20,
            "reason": "gateway S08 proof",
        },
    )
    assert rule["id"] == rule_id
    evaluated = gw_client.post(
        "/rules/evaluate",
        {
            "callback": "http.request",
            "subject": {"request": {"host": "probe.example.com"}},
        },
    )
    assert evaluated["matched_rule_id"] == rule_id
    assert evaluated["would_ask"] is True
    deleted_rule = gw_client.delete(f"/rules/{rule_id}")
    assert deleted_rule["removed"] is True

    pending = gw_client.get("/confirm/pending")
    assert pending == {
        "mode": "settings_profiles_v2",
        "pending": [],
        "pending_count": 0,
        "resolve_available": False,
        "resolve_owner": "S15-confirm-ux",
    }


def test_profile_selected_vm_create_response_preserves_pin_and_asset_state(gw_client):
    created = gw_client.post(
        "/provision",
        {
            "name": f"gateway-profile-{uuid.uuid4().hex[:8]}",
            "ram_mb": 2048,
            "cpus": 2,
            "profile_id": "everyday-work",
            "profile_revision": "2026.0520.1",
        },
    )

    assert created["profile_id"] == "everyday-work"
    assert created["profile_revision"] == "2026.0520.1"
    assert created["profile_pin"]["profile_payload_hash"].startswith("blake3:")
    assert created["profile_pin"]["package_contract_hash"].startswith("blake3:")
    assert created["profile_pin"]["base_assets"]["rootfs_hash"].startswith("c")
    assert created["asset_health"]["ready"] is True
    assert created["asset_health"]["profile_id"] == "everyday-work"


def test_gateway_status_preserves_profile_identity_and_asset_provenance(gw_client):
    status = gw_client.get("/status")

    assert status["service"] == "running"
    assert status["assets"]["profile_id"] == "everyday-work"
    assert status["assets"]["profile_revision"] == "2026.0520.1"
    assert status["assets"]["profile_payload_hash"].startswith("blake3:")
    assert status["assets"]["profile_assets"][0]["logical_name"] == "rootfs.squashfs"
    assert status["vms"][0]["profile_id"] == "everyday-work"
    assert status["vms"][0]["profile_status"] == "current"


def test_profile_asset_progress_proxies_setup_assets_envelope(gw_client):
    assets = gw_client.get("/setup/assets")

    assert assets["ready"] is False
    assert assets["state"] == "updating"
    assert assets["downloading"] is True
    assert assets["asset_version"] == "everyday-work@2026.0520.1"
    assert assets["profile_id"] == "everyday-work"
    assert assets["profile_revision"] == "2026.0520.1"
    assert assets["profile_payload_hash"].startswith("blake3:")
    assert assets["profile_assets"][0]["logical_name"] == "rootfs.squashfs"
    assert assets["missing"] == ["rootfs.squashfs"]
    assert assets["progress"] == {
        "logical_name": "rootfs.squashfs",
        "bytes_done": 6,
        "bytes_total": 12,
        "done": False,
    }
    assert {
        item["name"]: item["status"] for item in assets["assets"]
    }["rootfs.squashfs"] == "downloading"


def test_gateway_debug_report_preserves_profile_v2_provenance(gw_client):
    report = gw_client.get("/debug/report")

    assert report["json"]["schema"] == "capsem.debug.v2"
    assert report["json"]["assets"]["source"] == "profile_v2_asset_health"
    health = report["json"]["assets"]["health"]
    assert health["profile_id"] == "everyday-work"
    assert health["profile_revision"] == "2026.0520.1"
    assert health["profile_payload_hash"].startswith("blake3:")
    assert health["profile_assets"][0]["source_url"] == (
        "https://assets.example.test/rootfs.squashfs"
    )
    assert "profile_asset_profile_id: everyday-work" in report["text"]
    assert "vm_profile_pin: vm-001 everyday-work@2026.0520.1 current" in report["text"]


def test_gateway_preserves_profile_v2_typed_error_status_and_body(gw_client):
    status, body = gw_client.post_status_and_body(
        "/profiles/revoked-work/revisions/install",
        {},
    )
    parsed = json.loads(body)

    assert status == 409
    assert parsed == {
        "error": "profile revision is revoked",
        "mode": "settings_profiles_v2",
        "profile_id": "revoked-work",
        "revision": "2026.0301.1",
        "status": "revoked",
    }


@pytest.mark.parametrize(
    ("method", "path", "body", "expected_status", "expected_body"),
    [
        (
            "POST",
            "/profiles",
            {"version": 1, "name": "Missing ID"},
            400,
            {
                "mode": "settings_profiles_v2",
                "error": "profile validation failed: id is required",
                "code": "profile_invalid",
                "field": "id",
            },
        ),
        (
            "DELETE",
            "/skills/dev-sprint?kind=enabled",
            None,
            409,
            {
                "mode": "settings_profiles_v2",
                "error": "skill_is_locked: skill 'dev-sprint' is inherited from profile 'everyday-work'",
                "code": "skill_is_locked",
                "profile_id": "everyday-work",
                "skill_id": "dev-sprint",
                "kind": "enabled",
            },
        ),
        (
            "DELETE",
            "/mcp/connectors/builtin-local",
            None,
            409,
            {
                "mode": "settings_profiles_v2",
                "error": "server_is_locked: MCP server 'builtin-local' is inherited from profile 'everyday-work'",
                "code": "server_is_locked",
                "profile_id": "everyday-work",
                "connector_id": "builtin-local",
            },
        ),
        (
            "DELETE",
            "/rules/security.rules.http.default_read",
            None,
            409,
            {
                "mode": "settings_profiles_v2",
                "error": "rule_is_builtin: rule 'security.rules.http.default_read' is inherited from profile 'everyday-work'",
                "code": "rule_is_builtin",
                "profile_id": "everyday-work",
                "rule_id": "security.rules.http.default_read",
            },
        ),
        (
            "POST",
            "/rules/evaluate",
            {"callback": "bad.callback", "subject": {}},
            400,
            {
                "mode": "settings_profiles_v2",
                "error": "unsupported policy callback 'bad.callback'",
                "code": "rule_evaluate_invalid_callback",
                "callback": "bad.callback",
            },
        ),
        (
            "POST",
            "/setup/assets/cleanup",
            {},
            409,
            {
                "mode": "settings_profiles_v2",
                "error": "asset cleanup is blocked while assets are updating; retry once assets are ready",
                "code": "asset_cleanup_blocked",
                "asset_state": "updating",
            },
        ),
    ],
)
def test_gateway_preserves_profile_v2_adversarial_typed_errors(
    gw_client,
    method,
    path,
    body,
    expected_status,
    expected_body,
):
    status, raw_body = gw_client.request_status_and_body(method, path, body)

    assert status == expected_status
    assert json.loads(raw_body) == expected_body
