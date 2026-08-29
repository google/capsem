"""Lint one declared surface through the shared Citadel harness.

One entry point per surface, one report format, and every surface fails
closed. Which surfaces exist is `[[lint_surfaces]]` in config/gate.toml; what
each one reads and which tool reads it is below.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from collections.abc import Iterator
from pathlib import Path

from capsem_builder.gate import shellsurfaces

from . import lint_harness as lintharness
from .lint_harness import Outcome, Tool

SHELLCHECK_LINE = re.compile(
    r"^(?P<file>[^:]+):(?P<line>\d+):\d+: \w+: (?P<message>.*) \[(?P<code>SC\d+)\]$"
)
#: A relative markdown link or image target, ignoring anchors and URLs.
MARKDOWN_LINK = re.compile(r"!?\[[^\]]*\]\(<?([^)>\s#]+)")
#: A skill's bundled resources, which the skill format says live beside it:
#: `references/`, `agents/`, `scripts/`, `assets/`. Only checked under
#: `skills/`, because a backticked `scripts/...` in ordinary prose is a
#: repository-root path and resolving it against the document reports a
#: missing file that is simply elsewhere.
BACKTICK_REFERENCE = re.compile(
    r"`((?:references|agents|scripts|assets)/[A-Za-z0-9_./-]+\.[a-z]+)`"
)
SKILL_ROOT = "skills/"


def shell_tool(severity: str, exclude: str) -> Tool:
    argv = [
        "uv",
        "run",
        "--project",
        "build_system",
        "--frozen",
        "shellcheck",
        f"--severity={severity}",
        "--shell=bash",
        "--format=gcc",
    ]
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

    return Tool(
        "shellcheck",
        tuple(argv),
        parse,
        preamble="#!/usr/bin/env bash\nset -euo pipefail\n",
        suffix=".sh",
    )


def docker_tool(ignored: tuple[str, ...]) -> Tool:
    argv = [
        "uv",
        "run",
        "--project",
        "build_system",
        "--frozen",
        "hadolint",
        "--format",
        "json",
        "--no-fail",
    ]
    for code in ignored:
        argv += ["--ignore", code]

    def parse(stdout: str, _stderr: str) -> Iterator[tuple[str, int, str, str]]:
        for item in json.loads(stdout or "[]"):
            yield item["file"], int(item["line"]), item["code"], item["message"]

    return Tool("hadolint", tuple(argv), parse, suffix=".Dockerfile")


def _generated(root: Path, candidates: set[str]) -> set[str]:
    """Targets git ignores, which are build output rather than promises.

    `assets/manifest.json` is produced by the asset build and absent from a
    clean checkout, so a document naming it is describing a real artifact, not
    pointing at a file someone forgot to write. Asking git rather than listing
    directories means a new generated tree is covered the day it is ignored.
    """
    if not candidates:
        return set()
    listed = sorted(candidates)
    result = subprocess.run(
        ["git", "check-ignore", "--stdin"],
        cwd=root,
        input="\n".join(listed),
        capture_output=True,
        text=True,
        check=False,
    )
    return set(result.stdout.split())


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
    unused = set(known)
    findings: list[lintharness.Finding] = []
    pending: list[tuple[str, str, int]] = []
    checked = 0
    for name, text in lintharness.tracked_files(root, "*.md")():
        checked += 1
        source = root / name
        targets = {(m, MARKDOWN_LINK) for m in MARKDOWN_LINK.findall(text)}
        if name.startswith(SKILL_ROOT):
            targets |= {(m, BACKTICK_REFERENCE) for m in BACKTICK_REFERENCE.findall(text)}
        for target, _pattern in sorted(targets):
            if target.startswith(("http://", "https://", "mailto:", "/")):
                continue
            # Only targets that name a file. `./stack` in the docs is a
            # Starlight route, resolved by the site router and not by the
            # filesystem, so checking it reports a missing page that renders.
            if not Path(target).suffix:
                continue
            key = f"{name}|{target}"
            if key in known:
                unused.discard(key)
                continue
            # A bundled resource resolves beside its document; a path like
            # `build_system/packaging/macos/build-pkg.sh` quoted in a skill is a repository path
            # named in prose. Try both before calling it broken -- checking
            # only the first reported forty-five files that plainly exist.
            if (source.parent / target).exists() or (root / target).exists():
                continue
            line = next((n for n, body in enumerate(text.splitlines(), 1) if target in body), 1)
            pending.append((name, target, line))
    ignored = _generated(root, {target for _name, target, _line in pending})
    findings.extend(
        lintharness.Finding("markdown", name, line, "LINK", f"missing target: {target}")
        for name, target, line in pending
        if target not in ignored
    )

    # A stale entry is a ratchet that has stopped ratcheting: it suppresses a
    # finding nobody has any more, and reads as coverage.
    findings.extend(
        lintharness.Finding(
            "markdown",
            key.split("|")[0],
            1,
            "STALE",
            f"inventory entry no longer applies; remove it: {key}",
        )
        for key in sorted(unused)
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
            lintharness.run(
                "shell scripts", tool, lintharness.tracked_files(root, "*.sh"), on_disk=True
            ),
            lintharness.run(
                "workflow run: bodies",
                tool,
                lintharness.embedded(
                    lambda: shellsurfaces.workflow_bodies(root / ".github" / "workflows")
                ),
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
                lintharness.tracked_files(root, "build_system/docker/Dockerfile*"),
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
        root / "build_system" / "docker",
        root / "config" / "docker",
        lambda templates: shellsurfaces.rendered_templates(
            templates, root / "config" / "docker" / "image"
        ),
    )
