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
import sys
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]
BOUNDARY = gate_config.load(PROJECT_ROOT).boundary

#: Every declared source family, by the config key that declares it.
FAMILIES = ("scripts", "rust", "bench")

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

    A suffix-less file with a shebang counts too: packaging helpers are
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
        detail.append(
            f"  grew past their ratchet: { {p: f'{a} -> {b}' for p, (a, b) in grew.items()} }"
        )
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
    if rule.oversized_line_counts:
        target = next(iter(rule.oversized_line_counts))
        expected = "grew past their ratchet"
        inflated[target] += 1
    else:
        # A family with no debt -- `bench` -- has nothing to grow past a
        # ratchet, so the failure it must still produce is the other one: a
        # file crossing the ceiling for the first time. Skipping here would
        # leave the only clean family the only unexercised one.
        target = max(inflated, key=lambda path: inflated[path])
        expected = "newly over the ceiling"
        inflated[target] = rule.max_lines + 1

    monkeypatch.setattr(
        sys.modules[__name__],
        "_tracked_line_counts",
        lambda *_args, **_kwargs: inflated,
    )
    with pytest.raises(AssertionError, match=expected):
        test_oversized_sources_match_the_exact_debt_ratchet(family)


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


# ---------------------------------------------------------------------------
# Shell bodies: the same rule, asked of shell that lives inside other files.
# ---------------------------------------------------------------------------

SHELL_BODY_RATIONALE = """\
A shell body is a sequence of simple commands.

Control flow, multi-line string building and here-documents inside a YAML
`run:` or a Dockerfile `RUN` are a program that no test can call, no reviewer
reads in place, and -- until recently -- no linter looked at. `[boundary]`
already says this of justfile recipes; this asks it of the other two places
shell hides.

Simple is already the norm, so the rule is not an aspiration: across 274 bodies
the median is 3 executable lines and p90 is 14. Only the tail is a problem, and
the tail is where the release logic sits.

The fix for an entry is a script under `scripts/`, which ShellCheck already
lints and a test can call. Not a bigger parser: reaching for one is how four
extraction bugs got written before this rule existed.

Bodies come from `capsem_builder.gate.shellsurfaces`, the same extractor the shell
audit lints through, so the linter and the ceiling cannot disagree about what
a body is.
"""


def _shell_bodies() -> dict[str, str]:
    from capsem_builder.gate import shellsurfaces

    bodies = dict(shellsurfaces.workflow_bodies(PROJECT_ROOT / ".github" / "workflows"))
    bodies.update(
        shellsurfaces.dockerfile_bodies(
            PROJECT_ROOT / "build_system" / "docker",
            PROJECT_ROOT / "config" / "docker",
            lambda templates: shellsurfaces.rendered_templates(
                templates, PROJECT_ROOT / "config" / "docker" / "image"
            ),
        )
    )
    return bodies


def test_there_are_shell_bodies_to_measure() -> None:
    """Fail closed: an extractor returning nothing is not a pass."""
    assert len(_shell_bodies()) > 200, "shell body extraction looks truncated"


def test_oversized_shell_bodies_match_the_exact_debt_ratchet() -> None:
    from capsem_builder.gate import shellsurfaces

    rule = BOUNDARY.shell_bodies
    actual = {
        name: len(shellsurfaces.executable_lines(body))
        for name, body in _shell_bodies().items()
        if len(shellsurfaces.executable_lines(body)) > rule.max_lines
    }
    expected = dict(rule.oversized_line_counts)
    if actual == expected:
        return

    added = sorted(set(actual) - set(expected))
    grew = {n: (expected[n], v) for n, v in actual.items() if n in expected and v > expected[n]}
    shrank = {n: (expected[n], v) for n, v in actual.items() if n in expected and v < expected[n]}
    gone = sorted(set(expected) - set(actual))

    detail = [f"ceiling {rule.max_lines} executable lines"]
    if added:
        detail.append(f"  new bodies over the ceiling; move them to scripts/: {added}")
    if grew:
        detail.append(
            f"  grew past their ratchet: { {n: f'{a} -> {b}' for n, (a, b) in grew.items()} }"
        )
    if shrank:
        detail.append(
            "  shrank; lower the ratchet in the same change: "
            f"{ {n: f'{a} -> {b}' for n, (a, b) in shrank.items()} }"
        )
    if gone:
        detail.append(f"  now under the ceiling; remove the entry: {gone}")

    raise AssertionError(SHELL_BODY_RATIONALE + "\n" + "\n".join(detail))


def test_a_new_oversized_shell_body_is_caught(monkeypatch: pytest.MonkeyPatch) -> None:
    """Break it once: an unlisted body over the ceiling must fail."""
    rule = BOUNDARY.shell_bodies
    invented = dict(_shell_bodies())
    invented["invented.yaml:job:0:too much shell"] = "\n".join(
        f"echo line {n}" for n in range(rule.max_lines + 1)
    )
    monkeypatch.setitem(globals(), "_shell_bodies", lambda: invented)
    with pytest.raises(AssertionError, match="new bodies over the ceiling"):
        test_oversized_shell_bodies_match_the_exact_debt_ratchet()
