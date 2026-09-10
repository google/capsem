"""The route benchmark fixture must measure real, nonempty comparisons."""

from __future__ import annotations

from copy import deepcopy
from pathlib import Path

import pytest
from helpers.workspace_changes import assert_workspace_changes, seed_workspace_changes

VALID = {
    "checkpoint": "cp-10", "total": 3, "has_more": False,
    "changes": [{"path": f"{kind}.txt", "kind": kind} for kind in ("created", "deleted", "modified")],
}


def test_expected_comparison_is_accepted() -> None:
    assert_workspace_changes(VALID)


@pytest.mark.parametrize("mutation", ["empty", "wrong_checkpoint", "pagination", "wrong_kind"])
def test_incorrect_comparison_cannot_be_measured(mutation: str) -> None:
    payload = deepcopy(VALID)
    match mutation:
        case "empty":
            payload.update(total=0, changes=[])
        case "wrong_checkpoint":
            payload["checkpoint"] = "cp-0"
        case "pagination":
            payload["has_more"] = True
        case "wrong_kind":
            payload["changes"] = [{"path": "created.txt", "kind": "modified"}]
    with pytest.raises(AssertionError):
        assert_workspace_changes(payload)


def test_fixture_never_replaces_an_existing_registry(tmp_path: Path) -> None:
    registry = tmp_path / "persistent_registry.json"
    registry.write_text("existing registry")
    with pytest.raises(ValueError, match="isolated empty registry"):
        seed_workspace_changes(tmp_path, tmp_path / "profiles")
    assert registry.read_text() == "existing registry"
    assert not (tmp_path / "persistent").exists()
