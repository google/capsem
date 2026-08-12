#!/usr/bin/env python3
"""Lint one declared surface through the shared Citadel harness.

One entry point per surface, one report format, and every surface fails
closed. Which surfaces exist is `[[lint_surfaces]]` in config/gate.toml; what
each one reads and which tool reads it is below.
"""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Iterator
from pathlib import Path

from capsem.gate import lintharness, shellsurfaces
from capsem.gate.lintharness import Outcome, Tool

SHELLCHECK_LINE = re.compile(r"^(?P<file>[^:]+):(?P<line>\d+):\d+: \w+: (?P<message>.*) \[(?P<code>SC\d+)\]$")
#: A relative markdown link or image target, ignoring anchors and URLs.
MARKDOWN_LINK = re.compile(r"!?\[[^\]]*\]\(<?([^)>\s#]+)")
#: A skill-local reference. Only `references/`: a backticked `scripts/...` in
#: prose is a repository-root path, not a promise relative to this document,
#: and resolving it here reports a missing file that is simply elsewhere.
BACKTICK_REFERENCE = re.compile(r"`(references/[A-Za-z0-9_./-]+\.[a-z]+)`")


def shell_tool(severity: str, exclude: str) -> Tool:
    argv = ["uv", "run", "shellcheck", f"--severity={severity}", "--shell=bash", "--format=gcc"]
    if exclude:
        argv += ["--exclude", exclude]

    def parse(stdout: str, _stderr: str) -> Iterator[tuple[str, int, str, str]]:
        for line in stdout.splitlines():
            match = SHELLCHECK_LINE.match(line)
            if match:
                yield (
                    match["file"],
                    int(match["line"]),
                    match["code"],
                    match["message"],
                )

    return Tool("shellcheck", tuple(argv), parse, preamble="#!/usr/bin/env bash\nset -euo pipefail\n", suffix=".sh")


def docker_tool(ignored: tuple[str, ...]) -> Tool:
    argv = ["uv", "run", "hadolint", "--format", "json", "--no-fail"]
    for code in ignored:
        argv += ["--ignore", code]

    def parse(stdout: str, _stderr: str) -> Iterator[tuple[str, int, str, str]]:
        for item in json.loads(stdout or "[]"):
            yield item["file"], int(item["line"]), item["code"], item["message"]

    return Tool("hadolint", tuple(argv), parse, suffix=".Dockerfile")


def _known_missing(root: Path) -> set[str]:
    """Targets already inventoried as debt, keyed `document|target`."""
    import tomllib

    data = tomllib.loads((root / "config" / "gate.toml").read_text(encoding="utf-8"))
    return set(data.get("lint", {}).get("markdown", {}).get("known_missing_targets", {}))


def markdown_links(root: Path) -> Outcome:
    """Every relative link and promised reference must resolve.

    Not a style linter. The defect this catches is a document telling a reader
    -- often an agent -- to open something that is not there, which is worse
    than saying nothing: three skills promised `references/` files that do not
    exist, and the reader follows the instruction and finds nothing.
    """
    known = _known_missing(root)
    findings = []
    checked = 0
    for name, text in lintharness.tracked_files(root, "*.md")():
        checked += 1
        source = root / name
        targets = {(m, MARKDOWN_LINK) for m in MARKDOWN_LINK.findall(text)}
        targets |= {(m, BACKTICK_REFERENCE) for m in BACKTICK_REFERENCE.findall(text)}
        for target, _pattern in sorted(targets):
            if target.startswith(("http://", "https://", "mailto:", "/")):
                continue
            # Only targets that name a file. `./stack` in the docs is a
            # Starlight route, resolved by the site router and not by the
            # filesystem, so checking it reports a missing page that renders.
            if not Path(target).suffix:
                continue
            if f"{name}|{target}" in known:
                continue
            if not (source.parent / target).exists():
                line = next(
                    (n for n, body in enumerate(text.splitlines(), 1) if target in body), 1
                )
                findings.append(
                    lintharness.Finding("markdown", name, line, "LINK", f"missing target: {target}")
                )
    return Outcome("markdown", checked, tuple(findings))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("surface")
    parser.add_argument("--severity", default="warning")
    parser.add_argument("--exclude", default="")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()
    root = arguments.root.resolve()

    if arguments.surface == "shell":
        tool = shell_tool(arguments.severity, arguments.exclude)
        outcomes = [
            lintharness.run("shell scripts", tool, lintharness.tracked_files(root, "*.sh"), on_disk=True),
            lintharness.run(
                "workflow run: bodies",
                tool,
                lintharness.embedded(lambda: shellsurfaces.workflow_bodies(root / ".github" / "workflows")),
            ),
            lintharness.run(
                "Dockerfile RUN bodies",
                tool,
                lintharness.embedded(lambda: _docker_run_bodies(root)),
            ),
        ]
    elif arguments.surface == "dockerfile":
        outcomes = [
            lintharness.run(
                "Dockerfiles",
                docker_tool(tuple(c for c in arguments.exclude.split(",") if c)),
                lintharness.tracked_files(root, "docker/Dockerfile*"),
                on_disk=True,
            )
        ]
    elif arguments.surface == "markdown":
        outcomes = [markdown_links(root)]
    else:
        parser.error(f"unknown surface: {arguments.surface}")

    rendered, status = lintharness.report(outcomes)
    print(rendered)
    return status


def _docker_run_bodies(root: Path) -> dict[str, str]:
    return shellsurfaces.dockerfile_bodies(
        root / "docker",
        root / "config" / "docker",
        lambda templates: shellsurfaces.rendered_templates(
            templates, root / "config" / "docker" / "image"
        ),
    )


if __name__ == "__main__":
    raise SystemExit(main())
