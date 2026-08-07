"""Ruff and Ty as graph steps, with a policy the loader can reject.

Both lived behind one opaque `Call` delegating to a sequential function, and
the configuration behind them was raw CLI flags and arbitrary strings. It
worked. What it could not do is let the plan schedule, time, or name the three
checks independently -- so "the source gate took four minutes" resolved to one
line covering Ruff, strict Ty and relaxed Ty together, and a typo in a
held-back rule name was a flag the tool silently ignored.

The tool vocabularies stay open: third-party rule names change, so their
*grammar* is validated and the pinned tool decides what exists. Capsem's own
vocabulary is closed and uses enums elsewhere.
"""

from __future__ import annotations

import re
import subprocess
import tokenize
import tomllib
from collections import Counter
from pathlib import Path

import pydantic
import pytest

from capsem.gate import config as gate_config
from capsem.gate.harnessschema import LintConfig

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _policy(**overrides: object) -> LintConfig:
    base = {
        "python_roots": ("src", "scripts"),
        "strict_roots": ("src",),
        "error_on_warning": True,
        "ty_ratchet": {"invalid-assignment": 1},
        "suppression_budget": {
            "noqa": 1,
            "type_ignore": 1,
            "ty_ignore": 1,
            "ruff_global_ignore": 1,
            "ruff_per_file_ignore": 1,
            "justification": "Existing test debt that may only shrink.",
        },
    }
    return LintConfig.model_validate({**base, **overrides})


# ---------------------------------------------------------------------------
# The policy, validated where it is read rather than where it is used
# ---------------------------------------------------------------------------


def test_a_valid_policy_loads() -> None:
    assert _policy().strict_roots == ("src",)


@pytest.mark.parametrize(
    "roots",
    (
        ("src", "src"),
        ("/etc",),
        ("../elsewhere",),
        ("",),
    ),
    ids=("duplicate", "absolute", "escaping", "empty"),
)
def test_a_root_that_is_not_a_relative_tree_is_refused(roots: tuple[str, ...]) -> None:
    with pytest.raises(pydantic.ValidationError):
        _policy(python_roots=roots, strict_roots=())


def test_a_strict_root_outside_the_checked_roots_is_refused() -> None:
    """It would silently check nothing, which reads as passing."""
    with pytest.raises(pydantic.ValidationError):
        _policy(python_roots=("src",), strict_roots=("scripts",))


@pytest.mark.parametrize("rule", ("Invalid-Assignment", "invalid assignment", "", "x"))
def test_a_ratchet_entry_that_is_not_a_rule_name_is_refused(rule: str) -> None:
    """`ty` ignores an unknown `--ignore`, so a typo held nothing back and
    looked exactly like a rule that had been fixed."""
    with pytest.raises(pydantic.ValidationError):
        _policy(ty_ratchet={rule: 1})


def test_a_nonpositive_ratchet_count_is_refused() -> None:
    with pytest.raises(pydantic.ValidationError):
        _policy(ty_ratchet={"invalid-assignment": 0})


def test_the_semantic_option_is_configuration_and_the_flag_is_code() -> None:
    """`--error-on-warning` was stored as CLI text.

    A ty warning exits zero, so the option is load-bearing policy; how the
    pinned tool spells it is an implementation detail of the adapter.
    """
    assert CONFIG.lint.error_on_warning is True
    assert not hasattr(CONFIG.lint, "ty_flags")


# ---------------------------------------------------------------------------
# Three steps, not one opaque call
# ---------------------------------------------------------------------------


def _plan():
    import argparse

    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    return GateCommand.registry["lint"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()


def test_each_tool_is_its_own_step() -> None:
    labels = set(_plan().labels)

    assert {"python.ruff", "python.ty.strict", "python.ty.relaxed"} <= labels


def test_the_steps_are_independent_leaves() -> None:
    """Ruff does not gate Ty and neither gates the other.

    One sequential function meant a Ruff failure hid whatever Ty would have
    said, which is how a gate takes three rounds to go green.
    """
    plan = _plan()

    for label in ("python.ruff", "python.ty.strict", "python.ty.relaxed"):
        assert not plan.after_of(label), f"{label} waits for something it does not need"


def test_the_argv_each_step_issues() -> None:
    described = _plan().describe()

    assert "uv run ruff check ." in described
    assert "uv run ty check --error-on-warning src" in described


def test_strict_holds_nothing_back() -> None:
    """The whole point of the strict list: `src` passes every rule."""
    plan = _plan()
    strict = " ".join(plan.step_named("python.ty.strict").render())

    assert "--ignore" not in strict


def test_the_relaxed_step_holds_back_each_ratchet_rule_exactly_once() -> None:
    plan = _plan()
    relaxed = " ".join(plan.step_named("python.ty.relaxed").render())

    for rule in CONFIG.lint.ty_ratchet:
        assert relaxed.count(f"--ignore {rule}") == 1
    assert "src" not in relaxed.split("--ignore")[0].split()[4:], (
        "a strict root must not be re-checked with rules held back"
    )


def test_the_type_ratchet_records_every_diagnostic_count_exactly() -> None:
    """Debt may shrink deliberately, but it may never grow invisibly."""
    from capsem.gate.sourcechecks import ty_inventory_argv

    result = subprocess.run(
        ty_inventory_argv(CONFIG.lint.relaxed_roots),
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    counts = Counter(re.findall(r"(?:error|warning)\[([a-z][a-z0-9-]+)\]", result.stdout))

    assert counts == Counter(CONFIG.lint.ty_ratchet), (
        "Ty debt changed. Fix new diagnostics; for reductions, lower the exact "
        "count or remove the family from config/gate.toml.\n"
        f"expected: {dict(CONFIG.lint.ty_ratchet)}\nactual: {dict(sorted(counts.items()))}"
    )


def test_python_suppressions_and_exclusions_cannot_grow_unnoticed() -> None:
    """A local ignore is debt too, even when the aggregate Ty count is flat."""
    comments = (
        token.string
        for root in CONFIG.lint.python_roots
        for path in sorted((PROJECT_ROOT / root).rglob("*.py"))
        for token in tokenize.generate_tokens(
            iter(path.read_text(encoding="utf-8").splitlines(keepends=True)).__next__
        )
        if token.type == tokenize.COMMENT
    )
    sources = "\n".join(comments)
    project = tomllib.loads((PROJECT_ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    ruff = project["tool"]["ruff"]
    ruff_lint = ruff["lint"]
    ty = project["tool"].get("ty", {})
    budget = CONFIG.lint.suppression_budget

    assert len(re.findall(r"#\s*noqa\b", sources)) == budget.noqa
    assert len(re.findall(r"#\s*type:\s*ignore\b", sources)) == budget.type_ignore
    assert len(re.findall(r"#\s*ty:\s*ignore\b", sources)) == budget.ty_ignore
    assert len(ruff_lint.get("ignore", ())) == budget.ruff_global_ignore
    assert sum(len(rules) for rules in ruff_lint.get("per-file-ignores", {}).values()) == (
        budget.ruff_per_file_ignore
    )
    assert "exclude" not in ruff and "extend-exclude" not in ruff
    assert "exclude" not in ty


def test_two_failing_tools_both_report() -> None:
    """The reason these are steps: the plan aggregates independent failures.

    The sequential version collected failures by hand into a list, which is
    the same thing done once, by hand, in a place the graph cannot see.
    """
    from helpers.gate import RecordingJournal, RecordingRunner

    from capsem.gate.context import Context
    from capsem.gate.errors import GateError

    runner = RecordingRunner(PROJECT_ROOT, failures=["ruff check", "ty check"])
    plan = _plan()

    with pytest.raises(GateError) as raised:
        plan.run(Context(runner, CONFIG, journal=RecordingJournal()))

    message = str(raised.value)
    assert "python.ruff" in message
    assert "python.ty.strict" in message


def test_no_source_check_hides_behind_an_opaque_call() -> None:
    """Two `Call` wrappers delegated to one sequential function."""
    for module in ("lint.py", "audits.py"):
        source = (PROJECT_ROOT / "src/capsem/gate" / module).read_text(encoding="utf-8")
        assert "Call(" not in source, f"{module} still wraps a source check in one call"


def test_the_fast_phase_and_the_command_compose_the_same_fragment() -> None:
    """Two spellings of one gate is two things to keep in step.

    Phases are flat rather than nested -- `package.arm64.build` sits inside
    the glow-up phase under its own name -- so a fragment keeps its labels
    wherever it is composed.
    """
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    fast = set(gate_labels("test-fast"))

    assert {"python.ruff", "python.ty.strict", "python.ty.relaxed"} <= fast


def test_the_gate_never_changes_a_tracked_file_s_mode() -> None:
    """The source digest hashes `S_IMODE`, so a chmod is a source change.

    `initrd.repack` chmodded `guest/artifacts/{capsem-doctor,capsem-bench,
    snapshots}` to 0555 on every run. Git records them 100755 and does not
    track the write bit, so the change is invisible to `git status` and fatal
    to `source.verify`: a fresh clone records the digest at 755, the repack
    drops it to 555, and the run ends with "the gate changed the source
    working tree" sixty minutes later.

    It passed here only because the damage was already done -- the files had
    been 555 on disk since some earlier run, so the chmod was a no-op. That is
    this plan's third defect exactly: a cross-run leftover that a warm machine
    depends on and a clean checkout cannot supply.

    Asserted against git's own record rather than a constant, so it stays true
    if a file's executable bit legitimately changes.
    """
    import subprocess

    listed = subprocess.run(
        ["git", "ls-files", "-s"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout

    wrong: list[str] = []
    for line in listed.splitlines():
        recorded, _, rest = line.partition(" ")
        name = rest.split("\t", 1)[1]
        path = PROJECT_ROOT / name
        if recorded not in {"100644", "100755"} or path.is_symlink() or not path.is_file():
            continue
        executable = recorded == "100755"
        mode = path.stat().st_mode & 0o777
        if bool(mode & 0o111) != executable or not mode & 0o200:
            wrong.append(f"{name}: git {recorded}, disk {mode:04o}")

    assert not wrong, (
        "these tracked files' modes disagree with git, so the source digest "
        "recorded at `source.record` cannot match the one `source.verify` "
        "recomputes on a clean checkout:\n  " + "\n  ".join(wrong)
    )
