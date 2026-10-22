"""Case-insensitive filesystem aliases must not widen cache ownership."""

from pathlib import Path

import pytest
from capsem_builder.cache.models import PruneAction, PrunePlan
from capsem_builder.cache.operations import apply_prune

from build_system.tests.cache.test_scratch_runs import foreign_run, scratch


def test_forged_lowercase_alias_cannot_delete_an_uppercase_only_namespace(tmp_path: Path) -> None:
    paths = scratch(tmp_path)
    root = paths.stage("test-temp").parent
    uppercase = root / "BADC0FFE"
    payload = foreign_run(uppercase, "run-1") / "pytest/payload"
    plan = PrunePlan(
        generated_ns=1, reclaim_bytes=0, violations=(),
        actions=(PruneAction(
            stage_id="test-temp", key="badc0ffe/run-1", path=root / "badc0ffe/run-1",
            logical_bytes=0, reason="forged case alias",
        ),),
    )

    with pytest.raises(ValueError, match="outside cache stage"):
        apply_prune(paths, plan, reason="forged")

    assert payload.read_bytes() == b"y" * 64
    assert (uppercase / ".run-1.lock").is_file()
