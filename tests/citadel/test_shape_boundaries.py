"""Citadel guard: first-party source keeps a shape its tools can read.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is the shared root of several of them, so it owns every
source-size rule rather than leaving one per tree.

Why one file: `[boundary.scripts]` and `[boundary.rust]` are not two policies,
they are one rule asked of two trees. Two implementations of one rule is how
they drift, and drift is what a ratchet exists to prevent.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]
BOUNDARY = gate_config.load(PROJECT_ROOT).boundary

#: Every declared source family, by the config key that declares it.
FAMILIES = ("scripts", "rust")

SHAPE_RATIONALE = """\
First-party source must keep a shape its tools can read.

The rules themselves are stated where they are declared, and this does not
restate them -- a second wording is a second thing to keep in step:

    config/gate.toml [boundary]          recipe, module, script and Rust
                                         ceilings, each with its reason
    config/gate.toml [audits]            the skill description budget
    skills/dev-gate/SKILL.md             why the justfile/package boundary
                                         exists at all
    CLAUDE.md, "Code Style"              minimize code; every line earns its
                                         place

What this guard adds is the observation that they are one rule. Each ceiling
exists so a property stays provable, and each lapses the same way: something
grows past the point where a tool, a reviewer or a test can take it in, and
then nothing checks it. `capsem-service/src/main.rs` reached 13,851 lines in a
crate CLAUDE.md calls a thin shell, because Python had a ceiling and Rust had
none.

Two operating notes, because they are what people get wrong:

A ceiling is an outlier detector, not a rewrite mandate, so it comes from the
tree's own distribution. Rust's median tracked file is 232 lines; borrowing
Python's 300 would flag 43% of the tree and be deleted the first time it
blocked someone.

The inventory below a ceiling is exact debt. An entry may not grow, and a file
that shrinks updates or removes its entry in the same change -- an inventory
that drifts from the tree has stopped ratcheting.
"""


def _tracked_line_counts(
    roots: tuple[str, ...], suffixes: tuple[str, ...], *, root: Path = PROJECT_ROOT
) -> dict[str, int]:
    """Line counts for every tracked file in `roots` matching `suffixes`.

    A suffix-less file with a shebang counts too: `scripts/pkg-scripts/*` are
    programs whatever they are named, and a rule that missed them would be a
    rule anyone could route around by dropping an extension.
    """
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--", *roots],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")

    counts: dict[str, int] = {}
    for raw in listed:
        if not raw:
            continue
        relative = Path(raw.decode())
        source = (root / relative).read_text(encoding="utf-8")
        if relative.suffix in suffixes or source.startswith("#!"):
            counts[relative.as_posix()] = len(source.splitlines())
    return dict(sorted(counts.items()))


@pytest.mark.parametrize("family", FAMILIES)
def test_the_family_has_files_to_measure(family: str) -> None:
    """A ratchet over an empty tree asserts nothing."""
    rule = getattr(BOUNDARY, family)
    found = _tracked_line_counts(rule.roots, rule.suffixes)
    assert found, f"{family}: no tracked files under {rule.roots}; the rule is vacuous"


@pytest.mark.parametrize("family", FAMILIES)
def test_oversized_sources_match_the_exact_debt_ratchet(family: str) -> None:
    rule = getattr(BOUNDARY, family)
    actual = {
        path: lines
        for path, lines in _tracked_line_counts(rule.roots, rule.suffixes).items()
        if lines > rule.max_lines
    }
    expected = dict(rule.oversized_line_counts)
    if actual == expected:
        return

    grew = {p: (expected[p], n) for p, n in actual.items() if p in expected and n > expected[p]}
    added = sorted(set(actual) - set(expected))
    shrank = {p: (expected[p], n) for p, n in actual.items() if p in expected and n < expected[p]}
    gone = sorted(set(expected) - set(actual))

    detail = [f"{family}: ceiling {rule.max_lines}"]
    if added:
        detail.append(f"  newly over the ceiling: {added}")
    if grew:
        detail.append(f"  grew past their ratchet: { {p: f'{a} -> {b}' for p, (a, b) in grew.items()} }")
    if shrank:
        detail.append(
            "  shrank; lower the ratchet in the same change: "
            f"{ {p: f'{a} -> {b}' for p, (a, b) in shrank.items()} }"
        )
    if gone:
        detail.append(f"  now under the ceiling; remove the entry: {gone}")

    raise AssertionError(SHAPE_RATIONALE + "\n" + "\n".join(detail))


@pytest.mark.parametrize("family", FAMILIES)
def test_only_tracked_first_party_files_enter_the_inventory(family: str) -> None:
    """Every inventory entry names a file `git ls-files` actually reports.

    An entry for a path that no longer exists is a ratchet holding a file
    nobody has, which reads as coverage and is not.
    """
    rule = getattr(BOUNDARY, family)
    tracked = _tracked_line_counts(rule.roots, rule.suffixes)
    unknown = sorted(set(rule.oversized_line_counts) - set(tracked))
    assert not unknown, SHAPE_RATIONALE + f"\n{family}: inventory names untracked files: {unknown}"


# ---------------------------------------------------------------------------
# The guard's own tests. A ratchet nobody has watched fail is a ratchet nobody
# knows the shape of.
# ---------------------------------------------------------------------------


def test_only_tracked_first_party_programs_are_measured(tmp_path: Path) -> None:
    """Untracked, generated and vendored files stay out, by rule.

    `git ls-files` decides, not a glob: a generated file that happens to sit in
    `scripts/` is not first-party source, and a pattern-based rule would have
    to keep chasing new output paths.

    A suffix-less file with a shebang is still a program and is counted, so the
    ceiling cannot be avoided by dropping an extension.
    """
    (tmp_path / "scripts").mkdir()
    (tmp_path / "vendor").mkdir()
    (tmp_path / "scripts" / "module.py").write_text("one\ntwo\n", encoding="utf-8")
    (tmp_path / "scripts" / "installer").write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    (tmp_path / "scripts" / "data.xml").write_text("<data />\n", encoding="utf-8")
    (tmp_path / "scripts" / "generated.py").write_text("generated\n", encoding="utf-8")
    (tmp_path / "vendor" / "tool.py").write_text("vendored\n", encoding="utf-8")

    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    subprocess.run(
        ["git", "add", "scripts/module.py", "scripts/installer", "scripts/data.xml", "vendor"],
        cwd=tmp_path,
        check=True,
    )

    measured = _tracked_line_counts((("scripts",)), (".py", ".sh"), root=tmp_path)
    assert measured == {"scripts/installer": 2, "scripts/module.py": 2}


@pytest.mark.parametrize("family", FAMILIES)
def test_the_ratchet_notices_a_file_growing(family: str, monkeypatch: pytest.MonkeyPatch) -> None:
    """Break it once: an inventoried file gaining a line must fail.

    Asserted by substituting the measurement rather than by editing a real
    source file, so the guard is exercised without the test mutating the tree
    it is judging.
    """
    rule = getattr(BOUNDARY, family)
    inflated = dict(_tracked_line_counts(rule.roots, rule.suffixes))
    target = next(iter(rule.oversized_line_counts))
    inflated[target] += 1

    monkeypatch.setattr(
        "tests.citadel.test_shape_boundaries._tracked_line_counts",
        lambda *_args, **_kwargs: inflated,
        raising=False,
    )
    globals()["_tracked_line_counts"] = lambda *_a, **_k: inflated
    try:
        with pytest.raises(AssertionError, match="grew past their ratchet"):
            test_oversized_sources_match_the_exact_debt_ratchet(family)
    finally:
        globals()["_tracked_line_counts"] = _REAL_TRACKED_LINE_COUNTS


@pytest.mark.parametrize("family", FAMILIES)
def test_the_ratchet_notices_a_new_file_over_the_ceiling(family: str) -> None:
    """A file nobody inventoried appearing over the ceiling must fail."""
    rule = getattr(BOUNDARY, family)
    invented = dict(_tracked_line_counts(rule.roots, rule.suffixes))
    invented["invented/over-the-ceiling.py"] = rule.max_lines + 1

    globals()["_tracked_line_counts"] = lambda *_a, **_k: invented
    try:
        with pytest.raises(AssertionError, match="newly over the ceiling"):
            test_oversized_sources_match_the_exact_debt_ratchet(family)
    finally:
        globals()["_tracked_line_counts"] = _REAL_TRACKED_LINE_COUNTS


#: Captured before the negative tests swap it out.
_REAL_TRACKED_LINE_COUNTS = _tracked_line_counts
