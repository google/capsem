"""All cache deletion crosses one contained, journaled mutation boundary."""

import json
from pathlib import Path

import pytest
from capsem_builder.cache.models import PruneAction, PrunePlan
from capsem_builder.cache.operations import apply_prune


def plan(path: Path) -> PrunePlan:
    return PrunePlan(
        generated_ns=1,
        reclaim_bytes=3,
        actions=(
            PruneAction(
                stage_id="objects",
                key="old",
                path=path,
                logical_bytes=3,
                reason="over soft cap",
            ),
        ),
        violations=(),
    )


def test_apply_removes_only_planned_entries_and_journals_the_reason(tmp_path: Path) -> None:
    root = tmp_path / "cache"
    target = root / "target/objects/old"
    target.mkdir(parents=True)
    (target / "payload").write_bytes(b"abc")

    result = apply_prune(root, plan(target), reason="operator requested")

    assert not target.exists()
    assert result.removed == (target,)
    journal = root / "state/events/cache.jsonl"
    event = json.loads(journal.read_text(encoding="utf-8"))
    assert event["reason"] == "operator requested"
    assert event["removed"] == [str(target)]


def test_apply_refuses_a_target_outside_the_cache_root(tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.write_bytes(b"keep")

    with pytest.raises(ValueError, match="outside cache root"):
        apply_prune(tmp_path / "cache", plan(outside), reason="bad plan")

    assert outside.read_bytes() == b"keep"
