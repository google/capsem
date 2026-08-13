"""Citadel guard: every first-party surface has a check that can fail the run.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards against the mistake being *absence*: not a rule
broken, but a kind of file nobody ever pointed a tool at.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from helpers.gate import gate_plan

from capsem.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(PROJECT_ROOT)
SURFACES = CONFIG.lint_surfaces

#: The phases that answer before anything expensive starts.
FAST_PHASES = ("fast.", "python.")

LINT_COVERAGE_RATIONALE = """\
Every first-party surface must have a check that can fail the run.

The question is not "does this file pass" but "is anything looking at this kind
of file at all". That gap is invisible by construction: nothing fails, because
nothing runs.

It has already happened twice here. Shell went unchecked across 6,821 lines
while four `# shellcheck disable=` directives sat in the tree, written for a
linter no lane ran -- someone assumed coverage that did not exist. Markdown is
79,000 lines across 362 files with nothing pointed at it. Neither was a
decision; both surfaces simply arrived without a gate and nothing was watching.

`[[lint_surfaces]]` in config/gate.toml makes the absence visible. Each entry
names a kind of file and the steps that must check it, and this guard proves
three things:

  the surface exists            a rule for files nobody has is noise
  its checker is in the plan    a named step that no command builds is a lie
  its checker runs early        a check behind the asset build is a check that
                                reports after the expensive work it should
                                have preceded

Adding a language means adding its gate in the same change. That is the whole
rule, and it is cheaper than discovering the gap the way the last two were
discovered.

See config/gate.toml [[lint_surfaces]] and skills/dev-gate/SKILL.md.
"""


def _tracked(pattern: str) -> list[str]:
    return subprocess.run(
        ["git", "ls-files", "--", pattern],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.split()


def _plan_labels() -> set[str]:
    return set(gate_plan("candidate").labels)


def test_surfaces_are_declared() -> None:
    """A coverage guard over an empty declaration asserts nothing."""
    assert SURFACES, "no [[lint_surfaces]] declared; this contract would be vacuous"


@pytest.mark.parametrize("surface", SURFACES, ids=[s.name for s in SURFACES])
def test_the_surface_has_files(surface) -> None:
    """A declared surface with no files is a rule about nothing."""
    found = _tracked(surface.pattern)
    assert found, (
        LINT_COVERAGE_RATIONALE + f"\n{surface.name}: no tracked files match {surface.pattern!r}; "
        "remove the declaration or fix the pattern"
    )


@pytest.mark.parametrize("surface", SURFACES, ids=[s.name for s in SURFACES])
def test_the_surface_is_checked_by_a_step_that_exists(surface) -> None:
    labels = _plan_labels()
    missing = [step for step in surface.enforced_by if step not in labels]
    assert not missing, (
        LINT_COVERAGE_RATIONALE
        + f"\n{surface.name} ({len(_tracked(surface.pattern))} tracked files) names "
        f"steps no command builds: {missing}"
    )


@pytest.mark.parametrize("surface", SURFACES, ids=[s.name for s in SURFACES])
def test_the_surface_is_checked_before_the_expensive_work(surface) -> None:
    late = [step for step in surface.enforced_by if not step.startswith(FAST_PHASES)]
    assert not late, (
        LINT_COVERAGE_RATIONALE + f"\n{surface.name} is checked only outside the fast phase: {late}"
    )
