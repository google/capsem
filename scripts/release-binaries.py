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
import tomllib
from collections.abc import Sequence
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

try:
    from release_version_tag import (
        discard_claimed_version,
        ensure_version_tag,
        remote_version_target,
    )
except ModuleNotFoundError:
    from scripts.release_version_tag import (
        discard_claimed_version,
        ensure_version_tag,
        remote_version_target,
    )
CHANNELS = {"stable", "nightly"}
VERSION_COHORT_PATHS = (
    Path("Cargo.toml"),
    Path("Cargo.lock"),
    Path("crates/capsem-app/tauri.conf.json"),
    Path("pyproject.toml"),
    Path("uv.lock"),
)

SOURCE_COMMIT = re.compile(r"[0-9a-f]{40}\Z")
RUN_FIELDS = "databaseId,displayTitle,headSha,headBranch,status,conclusion"


@dataclass(frozen=True)
class CommandResult:
    stdout: str


@dataclass(frozen=True)
class ReleaseRun:
    database_id: str
    display_title: str
    head_sha: str
    head_branch: str
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


def _capture(runner: Runner, *argv: str) -> str:
    return runner.run(argv, capture=True).stdout.strip()


def _validate_start(runner: Runner, channel: str, source_commit: str) -> None:
    if channel not in CHANNELS:
        raise ValueError(f"channel must be stable or nightly, got {channel!r}")
    if SOURCE_COMMIT.fullmatch(source_commit) is None:
        raise ValueError("source commit must be 40-character lowercase hexadecimal")
    if _capture(runner, "git", "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("release-binaries requires a clean working tree")
    if _capture(runner, "git", "branch", "--show-current"):
        raise ValueError("release-binaries must run from a detached source commit")
    head = _capture(runner, "git", "rev-parse", "HEAD")
    if head != source_commit:
        raise ValueError(f"release source is {head}, expected {source_commit}")


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


# Independent of Cargo.toml so moving off the intended line is not tautological.
RELEASE_CONFIG = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))[
    "release"
]
RELEASE_LINE = RELEASE_CONFIG["line"]
SOURCE_REF_TEMPLATE = RELEASE_CONFIG["source_ref_template"]
TAGGER_NAME = RELEASE_CONFIG["tagger_name"]
TAGGER_EMAIL = RELEASE_CONFIG["tagger_email"]


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


def _matching_run(
    runner: Runner,
    ref: str,
    source_commit: str,
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
        "--commit",
        source_commit,
        "--event",
        "workflow_dispatch",
        "--limit",
        "20",
        "--json",
        "databaseId,displayTitle,headSha,headBranch,status,conclusion",
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
    display_title = row.get("displayTitle")
    head_sha = row.get("headSha")
    head_branch = row.get("headBranch")
    status = row.get("status")
    conclusion = row.get("conclusion")
    if (
        not isinstance(database_id, int)
        or not isinstance(display_title, str)
        or head_sha != source_commit
        or head_branch != ref
        or not isinstance(status, str)
    ):
        raise RuntimeError(f"GitHub returned a malformed matching release run: {row}")
    if conclusion is None:
        conclusion = ""
    if not isinstance(conclusion, str):
        raise RuntimeError(f"GitHub returned a malformed matching release conclusion: {row}")
    return ReleaseRun(str(database_id), display_title, head_sha, head_branch, status, conclusion)


def _wait_for_run(
    runner: Runner,
    ref: str,
    source_commit: str,
    tag: str,
    channel: str,
    dispatch_id: str,
) -> ReleaseRun:
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        run = _matching_run(runner, ref, source_commit, tag, channel, dispatch_id)
        if run is not None:
            return run
        time.sleep(2)
    raise RuntimeError(f"timed out waiting for release.yaml run for {channel}/{tag}")


def _dispatch_release(
    runner: Runner,
    *,
    ref: str,
    source_commit: str,
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
            "-f",
            f"source_commit={source_commit}",
        )
    )


def _complete_run(
    runner: Runner, run: ReleaseRun, tag: str, channel: str, source_commit: str, ref: str
) -> str:
    while True:
        if run.status != "completed":
            # A transient API 5xx kills the watcher, not the authoritative run.
            with suppress(subprocess.CalledProcessError):
                runner.run(("gh", "run", "watch", run.database_id, "--exit-status"))
        viewed = json.loads(
            _capture(runner, "gh", "run", "view", run.database_id, "--json", RUN_FIELDS)
        )
        status = viewed.get("status")
        conclusion = viewed.get("conclusion")
        identity_changed = (
            str(viewed.get("databaseId")) != run.database_id
            or viewed.get("displayTitle") != run.display_title,
            viewed.get("headSha") != source_commit,
            viewed.get("headBranch") != ref,
        )
        if any(identity_changed) or not isinstance(status, str) or not isinstance(conclusion, str):
            raise RuntimeError(f"release run identity changed: {viewed}")
        if status == "completed":
            break
        run = ReleaseRun(run.database_id, run.display_title, source_commit, ref, status, conclusion)
    if conclusion != "success":
        raise RuntimeError(
            f"{channel}/{tag} release run {run.database_id} completed with "
            f"{conclusion or 'an unknown conclusion'}; diagnose that run "
            "before retrying changed code"
        )
    return run.database_id


def _resume_release(
    runner: Runner,
    *,
    ref: str,
    source_commit: str,
    tag: str,
    channel: str,
    publish: bool,
    force_rebuild: bool = False,
) -> str:
    run = _matching_run(runner, ref, source_commit, tag, channel)
    if run is not None:
        if run.status != "completed" or run.conclusion != "success":
            return _complete_run(runner, run, tag, channel, source_commit, ref)
        if not force_rebuild:
            return _complete_run(runner, run, tag, channel, source_commit, ref)

    dispatch_id = f"capsem-binaries-{os.getpid()}-{time.time_ns()}"
    _dispatch_release(
        runner,
        ref=ref,
        source_commit=source_commit,
        tag=tag,
        channel=channel,
        publish=publish,
        dispatch_id=dispatch_id,
    )
    run = _wait_for_run(runner, ref, source_commit, tag, channel, dispatch_id)
    return _complete_run(runner, run, tag, channel, source_commit, ref)


def precheck_release_binaries(channel: str, source_commit: str, runner: Runner) -> None:
    """Validate immutable prepared source before the expensive proof."""
    _validate_start(runner, channel, source_commit)
    version = _project_version()
    _validate_version_cohort(version)


def release_binaries(channel: str, source_commit: str, runner: Runner) -> tuple[str, str]:
    _validate_start(runner, channel, source_commit)
    version = _project_version()
    _validate_version_cohort(version)
    tag = f"v{version}"
    claimed = remote_version_target(runner, tag) is None
    publish = ensure_version_tag(
        runner,
        tag=tag,
        channel=channel,
        source_commit=source_commit,
        tagger_name=TAGGER_NAME,
        tagger_email=TAGGER_EMAIL,
    )
    ref = SOURCE_REF_TEMPLATE.format(source_commit=source_commit)
    try:
        run_id = _resume_release(
            runner,
            ref=ref,
            source_commit=source_commit,
            tag=tag,
            channel=channel,
            publish=publish,
            force_rebuild=not publish,
        )
    except BaseException:
        if claimed:
            discard_claimed_version(runner, tag)
        raise
    return tag, run_id


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--precheck",
        action="store_true",
        help="check release notes only when a new immutable identity is needed",
    )
    parser.add_argument("channel", choices=sorted(CHANNELS))
    parser.add_argument("source_commit")
    args = parser.parse_args()
    try:
        runner = Runner()
        if args.precheck:
            precheck_release_binaries(args.channel, args.source_commit, runner)
            return 0
        tag, run_id = release_binaries(args.channel, args.source_commit, runner)
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
