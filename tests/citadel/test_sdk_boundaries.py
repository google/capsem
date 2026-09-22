"""SDK source, including checked-in generation, shares Citadel's size gates."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from citadel import test_shape_boundaries as shape

SDK_RATIONALE = (
    "SDK generation once produced a 7,559-line Python API and a 1,922-line "
    "TypeScript API outside the source guards. Checked-in generated code is "
    "maintained source: config/gate.toml [boundary] must measure it with the "
    "same ceilings and no new oversized-debt entries. Split or reduce it."
)

SDK_SOURCES = (
    ("scripts", "sdk/python/capsem/client.py"),
    ("scripts", "sdk/python/capsem/_generated/api.py"),
    ("scripts", "sdk/typescript/src/client.ts"),
    ("scripts", "sdk/typescript/src/generated/api.ts"),
    ("scripts", "sdk/typescript/scripts/check.mjs"),
    ("rust", "sdk/rust/src/generated/api.rs"),
)


def test_sdk_cannot_acquire_oversized_debt() -> None:
    debt = {
        name
        for family in shape.FAMILIES
        for name in getattr(shape.BOUNDARY, family).oversized_line_counts
        if Path(name).is_relative_to("sdk")
    }
    assert not debt, f"{SDK_RATIONALE}\nSDK debt entries: {sorted(debt)}"


@pytest.mark.parametrize(("family", "source"), SDK_SOURCES)
def test_sdk_size_inventory_and_rejection(
    family: str, source: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Exercise the real Git inventory and ratchet with oversized SDK files."""
    rule = getattr(shape.BOUNDARY, family)
    path = tmp_path / source
    path.parent.mkdir(parents=True)
    path.write_text("// generated source\n" * (rule.max_lines + 1))
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    subprocess.run(["git", "add", source], cwd=tmp_path, check=True)
    measured = shape._tracked_line_counts(rule.roots, rule.suffixes, root=tmp_path)
    assert measured == {source: rule.max_lines + 1}, SDK_RATIONALE

    # Retain the existing debt so rejection is specifically about this SDK file.
    monkeypatch.setattr(
        shape, "_tracked_line_counts",
        lambda *_args, **_kwargs: {**rule.oversized_line_counts, **measured},
    )
    with pytest.raises(AssertionError, match="newly over the ceiling"):
        shape.test_oversized_sources_match_the_exact_debt_ratchet(family)


def test_sdk_debt_guard_rejects_a_new_exception(monkeypatch: pytest.MonkeyPatch) -> None:
    rule = shape.BOUNDARY.scripts
    monkeypatch.setitem(rule.oversized_line_counts, "sdk/python/generated.py", 301)
    with pytest.raises(AssertionError, match="SDK debt entries"):
        test_sdk_cannot_acquire_oversized_debt()
