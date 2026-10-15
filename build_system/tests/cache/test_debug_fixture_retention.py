"""The fixture cache retains its working set after adding an old generation."""

from pathlib import Path

from capsem_builder.cache.config import load_policy
from capsem_builder.cache.models import CacheEntry, CacheInventory, StageInventory
from capsem_builder.cache.planner import plan_prune

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def test_prepared_debug_fixture_cohorts_survive_retention() -> None:
    # Measured pinned x86_64 fixture: 555258949 OCI bytes plus 680375257 bytes
    # of prepared EROFS/metadata. Four generations accommodate two checkouts
    # and both architectures. The old 1 GiB warm target could not keep even one.
    fixture_bytes = 1_235_634_206
    entries = tuple(
        CacheEntry(
            key=name,
            relative_path=Path("target/tests/images") / name,
            logical_bytes=fixture_bytes,
            allocated_bytes=fixture_bytes,
            created_ns=number,
            last_used_ns=number,
        )
        for number, name in enumerate(
            ("obsolete", "main-x86", "main-arm", "current-x86", "current-arm")
        )
    )
    total = fixture_bytes * len(entries)
    report = CacheInventory(
        root=Path("/repo/cache"),
        generated_ns=100,
        logical_bytes=total,
        allocated_bytes=total,
        stages=(
            StageInventory(
                stage_id="test-images",
                path=Path("/repo/cache/target/tests/images"),
                logical_bytes=total,
                allocated_bytes=total,
                protected_bytes=0,
                entries=entries,
            ),
        ),
    )

    plan = plan_prune(report, load_policy(PROJECT_ROOT))

    assert [action.key for action in plan.actions] == ["obsolete"], (
        "Cache retention must keep the pinned fixture working set, including "
        "prepared EROFS. Deleting every image causes repeated prefetch/boot failures."
    )
    assert not plan.violations
