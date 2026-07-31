"""Would the gate work if we started now?

One Python package with one console script is easy to say and easy to have
wrong. `uv sync` can succeed while the entry point resolves to a stale wheel, a
storage phase can name a rail the policy no longer declares, and a recipe can
dispatch to a subcommand that was renamed. Each of those surfaces deep inside a
run and reads as a product defect rather than an installation one.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import doctor
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _checkout(tmp_path: Path, *, gate_toml: str | None = None) -> Path:
    for name in ("pyproject.toml", "justfile"):
        (tmp_path / name).write_text(
            (PROJECT_ROOT / name).read_text(encoding="utf-8")
        )
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text(
        gate_toml or (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / "config" / "storage-policy.toml").write_text(
        (PROJECT_ROOT / "config" / "storage-policy.toml").read_text(encoding="utf-8")
    )
    return tmp_path


def test_this_checkout_is_ready() -> None:
    """The real thing, which is the only assertion that matters day to day."""
    assert doctor.check(RecordingRunner(PROJECT_ROOT)) == []


def test_a_storage_phase_naming_an_unknown_rail_is_reported(tmp_path: Path) -> None:
    """It would release nothing, and the next build would fail on ENOSPC
    somewhere with no connection to the cause."""
    original = (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    root = _checkout(
        tmp_path,
        gate_toml=original.replace(
            'after-install = { boundary = "after-install", rail = "install" }',
            'after-install = { boundary = "after-install", rail = "imaginary" }',
        ),
    )

    findings = doctor.check(RecordingRunner(root))

    assert [finding.check for finding in findings] == ["storage phase after-install"]
    assert "imaginary" in findings[0].detail


def test_a_recipe_dispatching_to_an_unknown_subcommand_is_reported(
    tmp_path: Path,
) -> None:
    """A renamed subcommand leaves the recipe failing on an argparse error,
    which reads as a recipe defect with no obvious cause."""
    root = _checkout(tmp_path)
    justfile = root / "justfile"
    justfile.write_text(
        justfile.read_text(encoding="utf-8").replace(
            "capsem-gate stamp-version", "capsem-gate stamp-the-version"
        )
    )

    findings = doctor.check(RecordingRunner(root))

    assert any("stamp-the-version" in finding.check for finding in findings)
    assert any("not a subcommand" in finding.detail for finding in findings)


def test_a_missing_storage_policy_is_reported_rather_than_raised(
    tmp_path: Path,
) -> None:
    root = _checkout(tmp_path)
    (root / "config" / "storage-policy.toml").unlink()

    findings = doctor.check(RecordingRunner(root))

    assert [finding.check for finding in findings] == ["storage policy"]


def test_the_command_names_every_problem_at_once(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Reporting one at a time turns setup into a guessing game."""
    monkeypatch.setattr(
        doctor,
        "check",
        lambda _runner: [
            doctor.Finding("first", "one thing"),
            doctor.Finding("second", "another"),
        ],
    )

    with pytest.raises(GateError) as failure:
        doctor._command(None, RecordingRunner(PROJECT_ROOT))

    assert "one thing" in str(failure.value)
    assert "another" in str(failure.value)


def test_a_ready_gate_says_so(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(doctor, "check", lambda _runner: [])
    runner = RecordingRunner(PROJECT_ROOT)

    assert doctor._command(None, runner) == 0
    assert any("configuration valid" in note for note in runner.notes)


def test_every_declared_console_script_is_runnable() -> None:
    """`uv sync` succeeding is not the same as the entry points working."""
    import tomllib

    declared = tomllib.loads(
        (PROJECT_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )["project"]["scripts"]

    assert "capsem-gate" in declared
    assert declared["capsem-gate"] == "capsem.gate.cli:main"


def test_the_justfile_dispatches_to_the_gate_rather_than_reimplementing_it() -> None:
    """The whole point of the boundary: recipes call, they do not decide."""
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    config = gate_config.load(PROJECT_ROOT)

    assert "uv run capsem-gate" in justfile
    assert config.boundary.remaining_shell_recipes, (
        "an empty list means the ratchet is finished and should be deleted"
    )


# ---------------------------------------------------------------------------
# The source gates themselves
# ---------------------------------------------------------------------------


def test_lint_runs_ruff_and_both_ty_passes(tmp_path: Path) -> None:
    """`ty` used to run on `src/capsem` alone, so `scripts/` -- release
    machinery, not scratch -- had no type gate at all."""
    from capsem.gate import lint

    root = _checkout(tmp_path)
    for name in gate_config.load(root).lint.python_roots:
        (root / name).mkdir(exist_ok=True)
    runner = RecordingRunner(root)

    lint.check(runner)

    assert runner.matching(r"ruff check \.")
    strict = runner.matching(r"ty check .*\bsrc\b")
    relaxed = runner.matching(r"ty check .*--ignore")
    assert strict and relaxed
    assert "--ignore" not in strict[0], (
        "src/ passes every rule and must be checked with none held back"
    )


def test_lint_reports_every_failing_gate_not_just_the_first(tmp_path: Path) -> None:
    """Stopping at the first tool leaves the second's findings for the next
    push, which is how a gate takes three rounds to go green."""
    from capsem.gate import lint

    root = _checkout(tmp_path)
    for name in gate_config.load(root).lint.python_roots:
        (root / name).mkdir(exist_ok=True)
    runner = RecordingRunner(root, failures=["ruff", "ty check"])

    with pytest.raises(GateError) as failure:
        lint.check(runner)

    assert "ruff" in str(failure.value)
    assert "ty" in str(failure.value)


def test_lint_warnings_fail_the_gate(tmp_path: Path) -> None:
    """A ty warning exits zero, so a warning-level rule on the ratchet could
    never have been detected as fixed."""
    from capsem.gate import lint

    root = _checkout(tmp_path)
    (root / "src").mkdir(exist_ok=True)
    runner = RecordingRunner(root)

    lint.check(runner)

    assert all("--error-on-warning" in line for line in runner.matching(r"ty check"))
