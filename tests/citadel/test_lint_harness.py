"""Citadel guard: a broken linter can never be reported as a clean surface."""

from __future__ import annotations

import json
import subprocess
from collections.abc import Iterator
from pathlib import Path

import pytest
from capsem_builder import crypto
from capsem_builder.gate.tools.audit import lint_harness as lintharness
from capsem_builder.gate.tools.audit.surfaces import markdown_links

LINT_PROCESS_RATIONALE = """\
The shared lint harness must distinguish "the tool found nothing" from "the
tool could not answer". A nonstandard exit or a findings exit whose output the
adapter cannot parse is missing evidence, not a clean surface. Treating either
as clean would make every Citadel lint guard fail open at the process boundary.
"""

UPSTREAM_DOCUMENT_RATIONALE = (
    "Only byte-identical upstream documents have upstream-relative links. "
    "First-party, changed, redirected and unpinned documents remain checked; "
    "a vendor directory alone never exempts a broken promise."
)


@pytest.mark.parametrize("kind", ["pinned", "changed", "unpinned", "first_party", "redirected"])
def test_markdown_upstream_paths_require_exact_document_provenance(tmp_path: Path, kind: str) -> None:
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "gate.toml").write_text("[lint]\n")
    vendor = tmp_path / "third_party" / "rust" / "fixture"
    vendor.mkdir(parents=True)
    original = "Read the [guide](guide.md).\n"
    document = vendor / "README.md"
    document.write_text(original)
    if kind != "unpinned":
        (vendor / "UPSTREAM.json").write_text(json.dumps({
            "files": {"README.md": crypto.sha256(original.encode()).hexdigest()},
        }))
    if kind == "changed":
        document.write_text("Changed. " + original)
    elif kind == "first_party":
        (tmp_path / "README.md").write_text(original)
    elif kind == "redirected":
        document.unlink()
        document.symlink_to(tmp_path / "original.md")
        (tmp_path / "original.md").write_text(original)
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True, timeout=10)
    subprocess.run(["git", "-C", str(tmp_path), "add", "config", "third_party"], check=True, timeout=10)
    if kind == "first_party":
        subprocess.run(["git", "-C", str(tmp_path), "add", "README.md"], check=True, timeout=10)
    findings = markdown_links(tmp_path).findings
    assert len(findings) == (0 if kind == "pinned" else 1), UPSTREAM_DOCUMENT_RATIONALE


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
