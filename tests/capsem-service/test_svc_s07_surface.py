"""S07 UDS service surface coverage."""

import uuid

import pytest

pytestmark = pytest.mark.integration


def test_confirm_pending_lists_typed_empty_surface(client):
    pending = client.get("/confirm/pending")

    assert pending == {
        "mode": "settings_profiles_v2",
        "pending": [],
        "pending_count": 0,
        "resolve_available": False,
        "resolve_owner": "S15-confirm-ux",
    }


def test_skills_add_list_delete_roundtrip(client):
    skill_id = f"pytest-{uuid.uuid4().hex[:8]}"

    created = client.post(
        "/skills",
        {
            "id": skill_id,
            "kind": "enabled",
        },
    )
    assert created["id"] == skill_id
    assert created["kind"] == "enabled"
    assert created["direct"] is True

    listed = client.get("/skills?kind=enabled")
    assert skill_id in listed["enabled"]
    by_id = {item["id"]: item for item in listed["skills"]}
    assert by_id[skill_id]["kind"] == "enabled"

    deleted = client.delete(f"/skills/{skill_id}?kind=enabled")
    assert deleted["skill_id"] == skill_id
    assert deleted["kind"] == "enabled"
    assert deleted["removed"] is True

    listed_after = client.get("/skills?kind=enabled")
    assert skill_id not in listed_after["enabled"]
