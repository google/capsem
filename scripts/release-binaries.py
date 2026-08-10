#!/usr/bin/env python3
"""Cut or rebuild one serialized Capsem binary release."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHANNELS = {"stable", "nightly"}
# Every file carrying the released version. Stamping propagates Cargo.toml's
# human-chosen version into the rest, so these change only when the cohort was
# not already in agreement -- which is why they are not required to change.
VERSION_COHORT_PATHS = (
    Path("Cargo.toml"),
    Path("Cargo.lock"),
    Path("crates/capsem-app/tauri.conf.json"),
    Path("pyproject.toml"),
    Path("uv.lock"),
)

# Produced from the Unreleased section on every release, so these must change.
RELEASE_NOTE_PATHS = (
    Path("CHANGELOG.md"),
    Path("LATEST_RELEASE.md"),
)

MUTATED_PATHS = (*VERSION_COHORT_PATHS, *RELEASE_NOTE_PATHS)


@dataclass(frozen=True)
class CommandResult:
    stdout: str


@dataclass(frozen=True)
class ReleaseRun:
    database_id: str
    status: str
    conclusion: str


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
            path: (root / path).read_bytes() if (root / path).exists() else None for path in paths
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


def _single_head_tag(runner: Runner) -> str | None:
    tags = [
        line
        for line in _capture(
            runner,
            "git",
            "tag",
            "--points-at",
            "HEAD",
            "--list",
            "v*",
        ).splitlines()
        if line
    ]
    if len(tags) > 1:
        raise ValueError(f"release HEAD has multiple immutable tags: {tags}")
    return tags[0] if tags else None


def _tag_channel(runner: Runner, tag: str) -> str | None:
    contents = _capture(
        runner,
        "git",
        "tag",
        "--list",
        tag,
        "--format=%(contents)",
    )
    match = re.search(r"(?:^|\s)channel=(stable|nightly)(?:\s|$)", contents)
    return match.group(1) if match is not None else None


def _validate_start(runner: Runner, channel: str) -> str | None:
    if channel not in CHANNELS:
        raise ValueError(f"channel must be stable or nightly, got {channel!r}")
    if _capture(runner, "git", "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("release-binaries requires a clean working tree")
    if _capture(runner, "git", "branch", "--show-current") != "main":
        raise ValueError("release-binaries must run from main")
    runner.run(("git", "fetch", "origin", "main"))
    head = _capture(runner, "git", "rev-parse", "HEAD")
    remote = _capture(runner, "git", "rev-parse", "origin/main")
    if head == remote:
        return None

    tag = _single_head_tag(runner)
    ahead = _capture(runner, "git", "rev-list", "--count", "origin/main..HEAD")
    parent = _capture(runner, "git", "rev-parse", "HEAD^")
    subject = _capture(runner, "git", "log", "-1", "--format=%s")
    expected_tag = f"v{_project_version()}"
    if (
        ahead == "1"
        and parent == remote
        and tag == expected_tag
        and _tag_channel(runner, tag) == channel
        and subject == f"release({channel}): {tag}"
    ):
        return tag
    raise ValueError(
        f"local main differs from origin/main and is not one resumable {channel} release commit"
    )


def _project_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "(\d+\.\d+\.\d+)"$', cargo, re.MULTILINE)
    if match is None:
        raise ValueError("Cargo.toml workspace version is missing or invalid")
    return match.group(1)


def _version_line(path: Path) -> str:
    content = path.read_text(encoding="utf-8")
    match = re.search(r'^version = "(\d+\.\d+\.\d+)"$', content, re.MULTILINE)
    if match is None:
        raise RuntimeError(f"release version cohort is missing a version in {path}")
    return match.group(1)


def _lock_package_versions(
    path: Path,
    *,
    package_name: str | None = None,
    package_prefix: str | None = None,
    workspace_only: bool = False,
) -> set[str]:
    versions: set[str] = set()
    for block in re.split(r"(?m)^\[\[package\]\]\s*$", path.read_text(encoding="utf-8")):
        name_match = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
        version_match = re.search(
            r'^version = "(\d+\.\d+\.\d+)"$',
            block,
            re.MULTILINE,
        )
        if name_match is None or version_match is None:
            continue
        name = name_match.group(1)
        if package_name is not None and name != package_name:
            continue
        if package_prefix is not None and not name.startswith(package_prefix):
            continue
        if workspace_only and re.search(r"^source = ", block, re.MULTILINE):
            continue
        versions.add(version_match.group(1))
    return versions


# The MAJOR.MINOR line this repository releases. Pinned deliberately rather
# than derived from the version: deriving it would make the cohort check
# tautological, and the point is to catch a version that moved off the line
# nobody intended to leave.
RELEASE_LINE = "0.6"


def _validate_version_cohort(version: str) -> None:
    tauri = json.loads((ROOT / "crates/capsem-app/tauri.conf.json").read_text(encoding="utf-8"))
    cohort = {
        "release line": {".".join(version.split(".")[:2])},
        "Cargo.toml": {version},
        "Cargo.lock workspace packages": _lock_package_versions(
            ROOT / "Cargo.lock",
            package_prefix="capsem",
            workspace_only=True,
        ),
        "tauri.conf.json": {str(tauri.get("version", ""))},
        "pyproject.toml": {_version_line(ROOT / "pyproject.toml")},
        "uv.lock capsem package": _lock_package_versions(
            ROOT / "uv.lock",
            package_name="capsem",
        ),
    }
    expected = {
        label: ({RELEASE_LINE} if label == "release line" else {version}) for label in cohort
    }
    mismatches = {
        label: sorted(values) for label, values in cohort.items() if values != expected[label]
    }
    if mismatches:
        raise RuntimeError(f"release version cohort is inconsistent for {version}: {mismatches}")


def _changed_paths(runner: Runner) -> set[Path]:
    output = runner.run(
        ("git", "status", "--porcelain", "--untracked-files=all"),
        capture=True,
    ).stdout.rstrip()
    return {Path(line[3:]) for line in output.splitlines() if len(line) >= 4}


def _matching_run(
    runner: Runner,
    ref: str,
    tag: str,
    channel: str,
    dispatch_id: str | None = None,
) -> ReleaseRun | None:
    raw = _capture(
        runner,
        "gh",
        "run",
        "list",
        "--workflow",
        "release.yaml",
        "--branch",
        ref,
        "--event",
        "workflow_dispatch",
        "--limit",
        "20",
        "--json",
        "databaseId,displayTitle,status,conclusion",
    )
    try:
        rows = json.loads(raw or "[]")
    except json.JSONDecodeError as error:
        raise RuntimeError(f"GitHub returned invalid release run JSON: {error}") from error
    if not isinstance(rows, list):
        raise RuntimeError("GitHub release run query did not return a list")
    title = f"Release {channel} {tag}"
    exact_title = f"{title} {dispatch_id}" if dispatch_id is not None else None
    matches = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        display_title = row.get("displayTitle")
        if exact_title is not None:
            matched = display_title == exact_title
        else:
            matched = display_title == title or (
                isinstance(display_title, str) and display_title.startswith(f"{title} ")
            )
        if matched:
            matches.append(row)
    if not matches:
        return None
    if dispatch_id is not None and len(matches) != 1:
        raise RuntimeError(f"GitHub returned multiple release runs for dispatch {dispatch_id}")
    row = matches[0]
    database_id = row.get("databaseId")
    status = row.get("status")
    conclusion = row.get("conclusion")
    if not isinstance(database_id, int) or not isinstance(status, str):
        raise RuntimeError(f"GitHub returned a malformed matching release run: {row}")
    if conclusion is None:
        conclusion = ""
    if not isinstance(conclusion, str):
        raise RuntimeError(f"GitHub returned a malformed matching release conclusion: {row}")
    return ReleaseRun(str(database_id), status, conclusion)


def _wait_for_run(
    runner: Runner,
    ref: str,
    tag: str,
    channel: str,
    dispatch_id: str,
) -> ReleaseRun:
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        run = _matching_run(runner, ref, tag, channel, dispatch_id)
        if run is not None:
            return run
        time.sleep(2)
    raise RuntimeError(f"timed out waiting for release.yaml run for {channel}/{tag}")


def _dispatch_release(
    runner: Runner,
    *,
    ref: str,
    tag: str,
    channel: str,
    publish: bool,
    dispatch_id: str,
) -> None:
    runner.run(
        (
            "gh",
            "workflow",
            "run",
            "release.yaml",
            "--ref",
            ref,
            "-f",
            f"tag={tag}",
            "-f",
            f"channel={channel}",
            "-f",
            f"publish={str(publish).lower()}",
            "-f",
            f"dispatch_id={dispatch_id}",
        )
    )


def _complete_run(runner: Runner, run: ReleaseRun, tag: str, channel: str) -> str:
    if run.status == "completed" and run.conclusion == "success":
        return run.database_id
    if run.status == "completed":
        raise RuntimeError(
            f"{channel}/{tag} release run {run.database_id} completed with "
            f"{run.conclusion or 'an unknown conclusion'}; diagnose that run "
            "before retrying changed code"
        )
    runner.run(("gh", "run", "watch", run.database_id, "--exit-status"))
    return run.database_id


def _resume_release(
    runner: Runner,
    *,
    ref: str,
    tag: str,
    channel: str,
    publish: bool,
    force_rebuild: bool = False,
) -> str:
    run = _matching_run(runner, ref, tag, channel)
    if run is not None:
        if run.status != "completed" or run.conclusion != "success":
            return _complete_run(runner, run, tag, channel)
        if not force_rebuild:
            return run.database_id

    dispatch_id = f"capsem-binaries-{os.getpid()}-{time.time_ns()}"
    _dispatch_release(
        runner,
        ref=ref,
        tag=tag,
        channel=channel,
        publish=publish,
        dispatch_id=dispatch_id,
    )
    run = _wait_for_run(runner, ref, tag, channel, dispatch_id)
    return _complete_run(runner, run, tag, channel)


def _tag_exists(runner: Runner, tag: str) -> bool:
    return _capture(runner, "git", "tag", "--list", tag) == tag


def precheck_release_binaries(channel: str, runner: Runner) -> None:
    """Require notes only when this invocation can mint a new identity."""
    if channel not in CHANNELS:
        raise ValueError(f"channel must be stable or nightly, got {channel!r}")
    expected_tag = f"v{_project_version()}"
    head_tag = _single_head_tag(runner)
    already_released = head_tag == expected_tag and (
        channel == "nightly" or _tag_channel(runner, head_tag) == channel
    )
    nightly_rebuild = channel == "nightly" and _tag_exists(runner, expected_tag)
    if already_released or nightly_rebuild:
        return
    if _tag_exists(runner, expected_tag):
        raise RuntimeError(
            f"immutable release tag already exists: {expected_tag}; bump the stable version"
        )
    runner.run((sys.executable, "scripts/extract-release-notes.py", "--check"))


def release_binaries(channel: str, runner: Runner) -> tuple[str, str]:
    pending_release_tag = _validate_start(runner, channel)
    if pending_release_tag is not None:
        runner.run(("git", "push", "--atomic", "origin", "main", pending_release_tag))
        run_id = _resume_release(
            runner,
            ref=pending_release_tag,
            tag=pending_release_tag,
            channel=channel,
            publish=True,
        )
        return pending_release_tag, run_id

    expected_tag = f"v{_project_version()}"
    current_release_tag = _single_head_tag(runner)
    if current_release_tag == expected_tag and (
        channel == "nightly" or _tag_channel(runner, current_release_tag) == channel
    ):
        publish = channel != "nightly"
        run_id = _resume_release(
            runner,
            ref=current_release_tag,
            tag=current_release_tag,
            channel=channel,
            publish=publish,
            force_rebuild=not publish,
        )
        return current_release_tag, run_id

    if channel == "nightly" and _tag_exists(runner, expected_tag):
        run_id = _resume_release(
            runner,
            ref="main",
            tag=expected_tag,
            channel=channel,
            publish=False,
            force_rebuild=True,
        )
        return expected_tag, run_id

    base_head = _capture(runner, "git", "rev-parse", "HEAD")
    runner.run(
        (
            sys.executable,
            "scripts/extract-release-notes.py",
            "--check",
        )
    )
    mutation = OwnedMutation(ROOT, MUTATED_PATHS)
    try:
        runner.run(("just", "_stamp-version"))
        version = _project_version()
        _validate_version_cohort(version)
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
        # Only the notes must change. A no-op stamp is the correct outcome when
        # the cohort already agrees on the released version, and that agreement
        # is proved above by _validate_version_cohort -- a stronger claim than
        # "the file was rewritten".
        missing = set(RELEASE_NOTE_PATHS) - changed
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
        runner.run(("git", "commit", "-m", f"release({channel}): {tag}"))
        runner.run(("git", "tag", "-a", tag, "-m", f"Capsem {version} channel={channel}"))
        mutation.committed = True
    except Exception:
        # Preparation starts from a validated clean tree and has not pushed.
        # A commit-hook or tag failure may nevertheless leave the release
        # cohort staged or HEAD advanced to an untagged commit. Restore the
        # exact local Git position first with a mixed reset (which preserves
        # unexpected files for inspection), then restore only our owned bytes.
        runner.run(("git", "reset", "--mixed", base_head))
        mutation.restore()
        raise

    runner.run(("git", "push", "--atomic", "origin", "main", tag))
    run_id = _resume_release(
        runner,
        ref=tag,
        tag=tag,
        channel=channel,
        publish=True,
    )
    return tag, run_id


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--precheck",
        action="store_true",
        help="check release notes only when a new immutable identity is needed",
    )
    parser.add_argument("channel", choices=sorted(CHANNELS))
    args = parser.parse_args()
    try:
        runner = Runner()
        if args.precheck:
            precheck_release_binaries(args.channel, runner)
            return 0
        tag, run_id = release_binaries(args.channel, runner)
    except (OSError, subprocess.CalledProcessError, RuntimeError, ValueError) as error:
        print(f"release-binaries failed: {error}", file=sys.stderr)
        return 1
    repository = os.environ.get("GITHUB_REPOSITORY")
    if repository:
        print(
            f"completed binary release/rebuild for {tag}: "
            f"https://github.com/{repository}/actions/runs/{run_id}"
        )
    else:
        print(f"completed binary release/rebuild for {tag}; GitHub Actions run {run_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
