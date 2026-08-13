"""Citadel guard: a broken linter can never be reported as a clean surface."""

from __future__ import annotations

import subprocess
from collections.abc import Iterator
from pathlib import Path

import pytest
from helpers.script_modules import load_script

lintharness = load_script(
    "citadel_lint_harness",
    Path(__file__).resolve().parents[2] / "scripts" / "lint_harness.py",
)

LINT_PROCESS_RATIONALE = """\
The shared lint harness must distinguish "the tool found nothing" from "the
tool could not answer". A nonstandard exit or a findings exit whose output the
adapter cannot parse is missing evidence, not a clean surface. Treating either
as clean would make every Citadel lint guard fail open at the process boundary.
"""


def _sources() -> Iterator[tuple[str, str]]:
    yield "fixture", "echo safe\n"


def _findings(_stdout: str, _stderr: str) -> Iterator[tuple[str, int, str, str]]:
    yield "fixture", 1, "L001", "finding"


def _empty(_stdout: str, _stderr: str) -> Iterator[tuple[str, int, str, str]]:
    yield from ()


@pytest.mark.parametrize(
    ("status", "parse"),
    ((2, _empty), (1, _empty)),
    ids=("tool-error", "unparseable-findings"),
)
def test_linter_failure_cannot_be_reported_clean(
    status: int,
    parse,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        lintharness.subprocess,
        "run",
        lambda *_args, **_kwargs: subprocess.CompletedProcess(
            args=["fixture-lint"], returncode=status, stdout="", stderr="tool failed"
        ),
    )
    tool = lintharness.Tool("fixture-lint", ("fixture-lint",), parse)

    with pytest.raises(RuntimeError, match="fixture-lint"):
        lintharness.run("fixture surface", tool, _sources)


def test_a_parseable_findings_exit_remains_a_normal_outcome(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        lintharness.subprocess,
        "run",
        lambda *_args, **_kwargs: subprocess.CompletedProcess(
            args=["fixture-lint"], returncode=1, stdout="fixture", stderr=""
        ),
    )
    tool = lintharness.Tool("fixture-lint", ("fixture-lint",), _findings)

    outcome = lintharness.run("fixture surface", tool, _sources)

    assert len(outcome.findings) == 1, LINT_PROCESS_RATIONALE


def test_sanitized_source_names_cannot_collapse_into_one_lint_input(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    invocations: list[list[str]] = []

    def completed(argv: list[str], **_kwargs) -> subprocess.CompletedProcess[str]:
        invocations.append(argv)
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(lintharness.subprocess, "run", completed)
    tool = lintharness.Tool("fixture-lint", ("fixture-lint",), _empty)

    def sources() -> Iterator[tuple[str, str]]:
        yield "a/b", "first\n"
        yield "a_b", "second\n"

    outcome = lintharness.run("fixture surface", tool, sources)

    assert outcome.checked == 2
    assert len(invocations) == 1
    assert len(invocations[0]) == 3, LINT_PROCESS_RATIONALE
    assert invocations[0][1] != invocations[0][2], LINT_PROCESS_RATIONALE
