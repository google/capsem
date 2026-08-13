"""One action-owned harness for every Citadel linter and report format.

Three linters over five source shapes arrived as three scripts with three
opinions about what a finding is, which is how a gate stops being comparable
with itself. This is the single spelling: a tool is declared in config, a
surface says which sources it reads, and every finding renders the same way.

The shapes differ and that is the whole difficulty. `*.sh` is a file on disk.
A workflow `run:` body lives inside YAML. A Dockerfile `RUN` body lives inside
an instruction, and for a `.j2` template it does not exist until rendered.
`Sources` is the seam: it yields `(name, text)` however it has to, and the
harness never learns which kind it was holding.

This lives under ``scripts/`` deliberately. It stages files and invokes tools,
so putting it inside ``capsem.gate`` would bypass that package's action-only
machine boundary. The gate records this whole program as one ``Run`` action.

Nothing here spells a source path, tool name or rule code. They come from
``[lint]`` and ``[[lint_surfaces]]`` in ``config/gate.toml``.
"""

from __future__ import annotations

import subprocess
import tempfile
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path

#: A source the harness can lint: a stable name, and the text to check.
Sources = Callable[[], Iterator[tuple[str, str]]]


@dataclass(frozen=True)
class Finding:
    """One linter result, in the one shape every tool is normalized into."""

    surface: str
    source: str
    line: int
    code: str
    message: str

    def render(self) -> str:
        return f"{self.surface}: {self.source}:{self.line} {self.code} {self.message}"


@dataclass(frozen=True)
class Outcome:
    """What one surface's lint run produced."""

    surface: str
    checked: int
    findings: tuple[Finding, ...]

    def render(self) -> str:
        verdict = "clean" if not self.findings else f"{len(self.findings)} findings"
        return f"{self.surface}: {self.checked} sources, {verdict}"


class EmptySurface(RuntimeError):
    """A surface produced no sources.

    Always an error, never a pass. "Found nothing so it was skipped" is how a
    gate stops being one, and an extractor that silently returns nothing is
    indistinguishable from a clean tree.
    """


class ToolFailure(RuntimeError):
    """A linter did not produce trustworthy evidence."""


def tracked_files(root: Path, pattern: str) -> Sources:
    """Sources from `git ls-files`, which decides what is first-party.

    Build output and vendored trees are excluded by rule rather than by
    pattern, so a new output directory cannot quietly enter the inventory.
    """

    def read() -> Iterator[tuple[str, str]]:
        listed = subprocess.run(
            ["git", "ls-files", "-z", "--", pattern],
            cwd=root,
            check=True,
            capture_output=True,
        ).stdout.split(b"\0")
        for raw in listed:
            if raw:
                relative = raw.decode()
                yield relative, (root / relative).read_text(encoding="utf-8")

    return read


def embedded(bodies: Callable[[], dict[str, str]]) -> Sources:
    """Sources that live inside another file and must be extracted first."""

    def read() -> Iterator[tuple[str, str]]:
        yield from bodies().items()

    return read


@dataclass(frozen=True)
class Tool:
    """A linter, and how to read what it says.

    `argv` is the command with no sources appended; `parse` turns its output
    into findings. Both come from the caller so this module never learns a
    tool's name or its flag spelling.
    """

    name: str
    argv: tuple[str, ...]
    parse: Callable[[str, str], Iterator[tuple[str, int, str, str]]]
    preamble: str = ""
    suffix: str = ""
    findings_statuses: tuple[int, ...] = (1,)
    """Exit statuses meaning findings were emitted rather than tool failure."""


def run(
    surface: str,
    tool: Tool,
    sources: Sources,
    *,
    on_disk: bool = False,
) -> Outcome:
    """Lint one surface and return findings in the shared shape.

    Extracted sources are staged in a temporary directory because linters read
    files, not strings. `on_disk` sources are passed through untouched so a
    finding names the real path rather than a scratch copy.
    """
    collected = list(sources())
    if not collected:
        raise EmptySurface(f"{surface}: no sources; refusing to pass vacuously")

    if on_disk:
        paths = {name: name for name, _text in collected}
        completed = _invoke(tool, list(paths.values()))
    else:
        with tempfile.TemporaryDirectory(prefix="capsem-lint-") as scratch:
            staging = Path(scratch)
            paths = {}
            for index, (name, text) in enumerate(collected):
                safe = "".join(c if c.isalnum() or c in "._-" else "_" for c in name)
                target = staging / f"{index:04d}-{safe}{tool.suffix}"
                target.write_text(tool.preamble + text, encoding="utf-8")
                paths[str(target)] = name
            completed = _invoke(tool, list(paths))

    allowed = (0, *tool.findings_statuses)
    if completed.returncode not in allowed:
        evidence = (completed.stderr or completed.stdout).strip()
        raise ToolFailure(
            f"{surface}: {tool.name} failed with status {completed.returncode}: {evidence}"
        )

    findings = tuple(
        Finding(surface, paths.get(source, source), line, code, message)
        for source, line, code, message in tool.parse(completed.stdout, completed.stderr)
    )
    if completed.returncode in tool.findings_statuses and not findings:
        evidence = (completed.stderr or completed.stdout).strip()
        raise ToolFailure(
            f"{surface}: {tool.name} exited for findings but its output could not be parsed: "
            f"{evidence}"
        )
    return Outcome(surface, len(collected), findings)


def _invoke(tool: Tool, sources: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [*tool.argv, *sources],
        check=False,
        capture_output=True,
        text=True,
    )


def report(outcomes: list[Outcome]) -> tuple[str, int]:
    """One report for every surface, and the exit status the gate should use."""
    lines = [outcome.render() for outcome in outcomes]
    findings = [finding for outcome in outcomes for finding in outcome.findings]
    if findings:
        lines.append("")
        lines.extend(finding.render() for finding in findings)
    return "\n".join(lines), (1 if findings else 0)
