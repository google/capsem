"""Citadel guard: every Rust target is linted, and every Rust test is run.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is the Rust half of the mistake `test_lint_coverage.py`
guards for other languages: not a rule broken, but a kind of code nobody points
a checker at. Nothing fails, because nothing runs.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import tomllib
from capsem_builder.gate import audits, rustchecks
from capsem_builder.gate import config as gate_config
from helpers.gate import gate_plan

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(PROJECT_ROOT)
MODULES = CONFIG.modules

#: Read from the one inventory rather than restated here. `[[lint_surfaces]]`
#: already records which steps must check each kind of file; a second list in
#: this file would be a second place to keep in step, and the first thing that
#: happens to a second list is that it stops matching.
RUST = next(surface for surface in CONFIG.lint_surfaces if surface.name == "rust")
CHECKERS = (*RUST.enforced_by, *RUST.checked_by)
CLIPPY_STEP = RUST.enforced_by[0]
FORMAT_STEP = "fast.rust-format"

RUST_COVERAGE_RATIONALE = """\
Every Rust target must be linted, and every Rust test must be run by something.

Three checkers divide this surface and each has an edge the others do not
reach, so a gap between them is invisible: the gate stays green while a whole
category stops being checked.

  clippy    --workspace --all-targets, or test and bench targets go unlinted
            while the lanes that run them stay green
  nextest   every native test target, under the `ci` profile whose
            `slow-timeout` bounds a hung test -- runs have died past the
            two-hour mark waiting on the 7200-second lock timeout instead
  --doc     doctests, which Nextest does not run and never will

That last one is the trap this guard exists for. `rustinventory` models native
and doctest targets separately and says so outright -- "Nextest never owns
doctests" -- so swapping the runner to Nextest without adding `cargo test
--doc` in the same change silently stops executing them. The gate goes faster
and proves less, and nothing anywhere reports a difference.

The doctest run sits beside the coverage run rather than in `fast`, and that is
deliberate: doctests need a built workspace, while `fast` holds nothing that
compiles -- clippy only checks. Moving it earlier would slow the phase whose
whole purpose is to answer in seconds. What belongs in `fast` is this guard.

Adding a Rust check means adding it here in the same change, or the shape stops
being provable.

See config/gate.toml [modules] and build_system/builder/gate/rustinventory.py.
"""

WORKSPACE_COVERAGE_RATCHETS = {
    "--fail-under-lines": 67.0,
    "--fail-under-functions": 66.0,
    "--fail-under-regions": 65.0,
}
MINIMUM_CRATE_COVERAGE = 40.0


def _doc_code_blocks() -> list[str]:
    """Files carrying a doc comment with a fenced code block.

    Source-level, and no `cargo metadata` subprocess: the Citadel runs in the
    fast phase beside Ruff, before the Rust toolchain step has necessarily
    installed anything, and a guard that needs cargo is a guard that fails for
    the wrong reason on a clean machine.
    """
    found = subprocess.run(
        ["git", "grep", "-l", "-e", "/// ```", "--", "crates/"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return found.stdout.split()


def _labels() -> set[str]:
    return set(gate_plan("candidate").labels)


def test_the_rust_tests_run_under_nextest() -> None:
    """The runner, and therefore the timeout policy, is the point."""
    assert "nextest" in MODULES.rust_coverage, (
        RUST_COVERAGE_RATIONALE
        + f"\nthe Rust test command is {' '.join(MODULES.rust_coverage)}; without "
        "nextest the `ci` profile's slow-timeout does not apply and a hung test "
        "hangs the gate until the machine lock times out"
    )


def test_the_timeout_profile_is_selected() -> None:
    """Nextest reads it from the environment; a flag would be cargo's."""
    assert MODULES.rust_test_profile and MODULES.rust_test_profile_variable, (
        RUST_COVERAGE_RATIONALE + "\nno nextest profile is configured, so the timeout policy in "
        ".config/nextest.toml is written but never selected"
    )


def test_doctests_are_run_by_something() -> None:
    """The trap: Nextest does not run them, so the runner swap drops them.

    Conditional on the repository actually having doctests, so this stays a
    statement about reality rather than a rule about nothing.
    """
    carrying = _doc_code_blocks()
    if not carrying:
        pytest.skip("no doc comments carry code blocks; nothing to run")

    assert any("doctest" in step for step in CHECKERS), (
        RUST_COVERAGE_RATIONALE
        + "\nthe rust surface names no doctest step in [[lint_surfaces]]"
    )
    assert MODULES.rust_doctests, (
        RUST_COVERAGE_RATIONALE
        + f"\n{len(carrying)} file(s) carry doctests and no command runs them: "
        + ", ".join(sorted(carrying))
    )
    assert "--doc" in MODULES.rust_doctests, (
        RUST_COVERAGE_RATIONALE
        + f"\nthe doctest command is {' '.join(MODULES.rust_doctests)}, which does "
        "not select doctests"
    )


def test_clippy_covers_every_target() -> None:
    """`--all-targets`, or tests and benches are linted by nothing."""
    rendered = " ".join(audits.clippy(CONFIG).render())
    for flag in ("--workspace", "--all-targets"):
        assert flag in rendered, (
            RUST_COVERAGE_RATIONALE + f"\nclippy runs without {flag}: {rendered}"
        )


def test_rust_format_covers_the_workspace() -> None:
    rendered = " ".join(CONFIG.modules.rust_format)
    assert FORMAT_STEP in RUST.enforced_by, (
        RUST_COVERAGE_RATIONALE + f"\n{FORMAT_STEP} does not enforce the Rust surface"
    )
    assert rendered == "cargo fmt --all -- --check", (
        RUST_COVERAGE_RATIONALE + f"\nRust format command drifted: {rendered}"
    )


def test_per_crate_coverage_ratchets_match_the_workspace() -> None:
    expected = {
        tomllib.loads(manifest.read_text())["package"]["name"]
        for manifest in (PROJECT_ROOT / MODULES.rust_coverage_crate_root).glob("*/Cargo.toml")
    }
    configured = set(MODULES.rust_coverage_crate_floors)
    assert configured == expected, (
        RUST_COVERAGE_RATIONALE
        + "\nper-crate coverage floor inventory drifted: "
        + f"missing={sorted(expected - configured)}, stale={sorted(configured - expected)}"
    )
    assert MODULES.rust_coverage_crate_minimum >= MINIMUM_CRATE_COVERAGE, (
        RUST_COVERAGE_RATIONALE
        + f"\nper-crate minimum fell below {MINIMUM_CRATE_COVERAGE:.0f}%: "
        + f"{MODULES.rust_coverage_crate_minimum:.2f}%"
    )
    below_minimum = {
        crate: floor
        for crate, floor in MODULES.rust_coverage_crate_floors.items()
        if floor < MODULES.rust_coverage_crate_minimum
    }
    assert not below_minimum, (
        RUST_COVERAGE_RATIONALE
        + "\ncrate floors below the configured workspace minimum: "
        + repr(below_minimum)
    )


def test_workspace_coverage_ratchets_every_llvm_dimension() -> None:
    configured = {}
    for floor in MODULES.rust_coverage_floors:
        name, separator, value = floor.partition("=")
        assert separator and value, (
            RUST_COVERAGE_RATIONALE + f"\nmalformed Rust coverage floor: {floor}"
        )
        configured[name] = float(value)

    assert configured.keys() == WORKSPACE_COVERAGE_RATCHETS.keys(), (
        RUST_COVERAGE_RATIONALE
        + "\nworkspace coverage must independently ratchet lines, functions, and regions: "
        + repr(configured)
    )
    regressions = {
        metric: (configured[metric], minimum)
        for metric, minimum in WORKSPACE_COVERAGE_RATCHETS.items()
        if configured[metric] < minimum
    }
    assert not regressions, (
        RUST_COVERAGE_RATIONALE
        + "\nworkspace coverage ratchets moved backwards: "
        + repr(regressions)
    )


def test_coverage_run_emits_and_checks_one_owned_report() -> None:
    rendered = "\n".join(action.render() for action in rustchecks.coverage(CONFIG).actions)
    assert "--lcov" in MODULES.rust_coverage, RUST_COVERAGE_RATIONALE
    assert MODULES.rust_coverage_report in MODULES.rust_coverage, (
        RUST_COVERAGE_RATIONALE + "\ncoverage reports to a path the ratchet does not read"
    )
    assert MODULES.rust_coverage_report in rendered, RUST_COVERAGE_RATIONALE
    assert MODULES.rust_coverage_ratchet in rendered, (
        RUST_COVERAGE_RATIONALE + "\nthe per-crate ratchet is configured but never executed"
    )
    for floor in MODULES.rust_coverage_floors:
        assert floor in rendered, (
            RUST_COVERAGE_RATIONALE + f"\nconfigured floor is not executed: {floor}"
        )


@pytest.mark.parametrize("label", CHECKERS)
def test_each_checker_is_in_the_plan(label: str) -> None:
    """A configured command that no plan builds is a check nobody runs."""
    assert label in _labels(), (
        RUST_COVERAGE_RATIONALE + f"\n{label} is not in the candidate plan"
    )


def test_the_lint_gate_answers_in_the_fast_phase() -> None:
    """Clippy is the one of the three that needs no build, so it goes early."""
    assert CLIPPY_STEP.startswith("fast."), (
        RUST_COVERAGE_RATIONALE + f"\n{CLIPPY_STEP} is no longer a fast-phase step"
    )


# -- adversarial: the detector has to see what it claims to ----------------


def test_the_doctest_detector_finds_the_real_ones() -> None:
    """It is the premise of `test_doctests_are_run_by_something`.

    A detector that silently found nothing would turn that test into a skip,
    which reads as "no doctests here" rather than "the guard broke".
    """
    carrying = _doc_code_blocks()
    assert carrying, "the doctest detector found nothing in a tree that has doctests"
    assert all(name.endswith(".rs") for name in carrying), carrying
