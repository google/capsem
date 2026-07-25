#!/usr/bin/env python3
"""Cut and dispatch one serialized Capsem binary release."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]
CHANNELS = {"stable", "nightly"}
MUTATED_PATHS = (
    Path("Cargo.toml"),
    Path("Cargo.lock"),
    Path("crates/capsem-app/tauri.conf.json"),
    Path("pyproject.toml"),
    Path("uv.lock"),
    Path("CHANGELOG.md"),
    Path("LATEST_RELEASE.md"),
)


@dataclass(frozen=True)
class CommandResult:
    stdout: str


class Runner:
    def run(
        self,
        argv: Sequence[str],
        *,
        capture: bool = False,
        env: dict[str, str] | None = None,
    ) -> CommandResult:
        completed = subprocess.run(
            list(argv),
            cwd=ROOT,
            check=True,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            env=env,
        )
        return CommandResult(stdout=completed.stdout or "")


class OwnedMutation:
    """Restore only release-owned files when preparation fails before commit."""

    def __init__(self, root: Path, paths: Sequence[Path]) -> None:
        self._snapshots = {
            path: (root / path).read_bytes() if (root / path).exists() else None
            for path in paths
        }
        self._root = root
        self.committed = False

    def restore(self) -> None:
        if self.committed:
            return
        for relative, content in self._snapshots.items():
            path = self._root / relative
            if content is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(content)


def _capture(runner: Runner, *argv: str) -> str:
    return runner.run(argv, capture=True).stdout.strip()


def _validate_start(runner: Runner, channel: str) -> None:
    if channel not in CHANNELS:
        raise ValueError(f"channel must be stable or nightly, got {channel!r}")
    if _capture(runner, "git", "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("release-binaries requires a clean working tree")
    if _capture(runner, "git", "branch", "--show-current") != "main":
        raise ValueError("release-binaries must run from main")
    runner.run(("git", "fetch", "origin", "main"))
    head = _capture(runner, "git", "rev-parse", "HEAD")
    remote = _capture(runner, "git", "rev-parse", "origin/main")
    if head != remote:
        raise ValueError("local main must exactly match origin/main before release")


def _project_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "(\d+\.\d+\.\d+)"$', cargo, re.MULTILINE)
    if match is None:
        raise ValueError("Cargo.toml workspace version is missing or invalid")
    return match.group(1)


def _changed_paths(runner: Runner) -> set[Path]:
    output = runner.run(
        ("git", "status", "--porcelain", "--untracked-files=all"),
        capture=True,
    ).stdout.rstrip()
    return {Path(line[3:]) for line in output.splitlines() if len(line) >= 4}


def _wait_for_run(runner: Runner, tag: str) -> str:
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        run_id = _capture(
            runner,
            "gh",
            "run",
            "list",
            "--workflow",
            "release.yaml",
            "--branch",
            tag,
            "--event",
            "workflow_dispatch",
            "--limit",
            "1",
            "--json",
            "databaseId",
            "--jq",
            ".[0].databaseId",
        )
        if run_id:
            return run_id
        time.sleep(2)
    raise RuntimeError(f"timed out waiting for release.yaml run for {tag}")


def release_binaries(
    channel: str, runner: Runner
) -> tuple[str | None, str | None]:
    _validate_start(runner, channel)
    current_release_tag = _capture(
        runner,
        "git",
        "tag",
        "--points-at",
        "HEAD",
        "--list",
        "v*",
    )
    if channel == "nightly" and current_release_tag:
        return None, None

    mutation = OwnedMutation(ROOT, MUTATED_PATHS)
    try:
        runner.run(("just", "_stamp-version"))
        version = _project_version()
        tag = f"v{version}"
        runner.run(
            (
                sys.executable,
                "scripts/extract-release-notes.py",
                "--version",
                version,
            )
        )
        changed = _changed_paths(runner)
        expected = set(MUTATED_PATHS)
        unexpected = changed - expected
        missing = {
            Path("Cargo.toml"),
            Path("crates/capsem-app/tauri.conf.json"),
            Path("pyproject.toml"),
            Path("uv.lock"),
            Path("CHANGELOG.md"),
            Path("LATEST_RELEASE.md"),
        } - changed
        if unexpected or missing:
            raise RuntimeError(
                "release preparation write set is invalid: "
                f"unexpected={sorted(map(str, unexpected))} "
                f"missing={sorted(map(str, missing))}"
            )
        existing = _capture(runner, "git", "tag", "--list", tag)
        if existing:
            raise RuntimeError(f"immutable release tag already exists: {tag}")
        runner.run(("git", "add", "--", *(str(path) for path in MUTATED_PATHS)))
        runner.run(("git", "commit", "-m", f"release: {tag}"))
        runner.run(("git", "tag", "-a", tag, "-m", f"Capsem {version}"))
        mutation.committed = True
    except Exception:
        mutation.restore()
        raise

    runner.run(("git", "push", "--atomic", "origin", "main", tag))
    runner.run(
        (
            "gh",
            "workflow",
            "run",
            "release.yaml",
            "--ref",
            tag,
            "-f",
            f"tag={tag}",
            "-f",
            f"channel={channel}",
        )
    )
    run_id = _wait_for_run(runner, tag)
    runner.run(("gh", "run", "watch", run_id, "--exit-status"))
    return tag, run_id


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("channel", choices=sorted(CHANNELS))
    args = parser.parse_args()
    try:
        tag, run_id = release_binaries(args.channel, Runner())
    except (OSError, subprocess.CalledProcessError, RuntimeError, ValueError) as error:
        print(f"release-binaries failed: {error}", file=sys.stderr)
        return 1
    if tag is None:
        print("nightly release skipped: current main has no unreleased binary changes")
        return 0
    repository = os.environ.get("GITHUB_REPOSITORY")
    if repository:
        print(f"released {tag}: https://github.com/{repository}/actions/runs/{run_id}")
    else:
        print(f"released {tag}; GitHub Actions run {run_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
