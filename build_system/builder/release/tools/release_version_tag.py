#!/usr/bin/env python3
"""Release-owned lifecycle of an immutable `v*` tag, claim through release.

A binary release has to push its version tag before it dispatches, because CI
attaches the release object to that tag. That ordering is what makes the tag a
claim rather than a record, and a claim needs a way to be given back: nothing
undid the tag when the build failed, so a single failed job burned the version
outright. The immutability guard then refused to reuse a tag pointing at a
different commit, and the next attempt had to invent a number for work that
never shipped.

Kept out of `release-binaries.py` because that script is already far past the
script ceiling, and because claiming, verifying and releasing a version is one
subject rather than four helpers that happen to sit near each other. The runner
arrives as a protocol so this module shares no infrastructure with its caller.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections.abc import Sequence
from contextlib import suppress
from typing import Any, Protocol


class TagRunner(Protocol):
    """Whatever the caller uses to run git and gh."""

    def run(
        self,
        argv: Sequence[str],
        *,
        capture: bool = False,
        env: dict[str, str] | None = None,
    ) -> Any: ...


def _capture(runner: TagRunner, *argv: str) -> str:
    return runner.run(argv, capture=True).stdout.strip()


def remote_version_target(runner: TagRunner, tag: str) -> str | None:
    """The commit the remote tag resolves to, or None when it does not exist.

    Both the tag ref and its peeled target are requested, so an annotated tag
    reports the commit it points at rather than its own object id.
    """
    raw = _capture(
        runner,
        "git",
        "ls-remote",
        "--tags",
        "origin",
        f"refs/tags/{tag}",
        f"refs/tags/{tag}^{{}}",
    )
    rows = [line.split() for line in raw.splitlines() if line]
    if not rows:
        return None
    if any(len(row) != 2 for row in rows):
        raise RuntimeError(f"remote returned malformed rows for {tag}: {rows}")
    targets = {row[1]: row[0] for row in rows}
    if len(targets) != len(rows):
        raise RuntimeError(f"remote returned duplicate rows for {tag}: {rows}")
    expected = {f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"}
    if set(targets) - expected or f"refs/tags/{tag}" not in targets:
        raise RuntimeError(f"remote returned malformed rows for {tag}: {rows}")
    return targets.get(f"refs/tags/{tag}^{{}}", targets[f"refs/tags/{tag}"])


def ensure_version_tag(
    runner: TagRunner,
    *,
    tag: str,
    channel: str,
    source_commit: str,
    tagger_name: str,
    tagger_email: str,
) -> bool:
    """Return whether this invocation may publish the selected binary identity."""
    target = remote_version_target(runner, tag)
    if target is None:
        runner.run(
            (
                "git",
                "-c",
                f"user.name={tagger_name}",
                "-c",
                f"user.email={tagger_email}",
                "tag",
                "-a",
                tag,
                source_commit,
                "-m",
                f"Capsem {tag[1:]} channel={channel}",
            )
        )
        runner.run(("git", "push", "origin", f"refs/tags/{tag}:refs/tags/{tag}"))
        if remote_version_target(runner, tag) != source_commit:
            raise RuntimeError(f"new immutable version tag {tag} did not resolve to {source_commit}")
        return True
    if target == source_commit:
        return True
    if channel == "nightly":
        return False
    raise RuntimeError(f"immutable version tag {tag} points at {target}, not {source_commit}")


def release_exists(runner: TagRunner, tag: str) -> bool:
    """Whether GitHub already has a release object behind this tag."""
    try:
        return bool(_capture(runner, "gh", "release", "view", tag, "--json", "tagName"))
    except subprocess.CalledProcessError:
        return False


def discard_claimed_version(runner: TagRunner, tag: str) -> None:
    """Give back a version this invocation claimed and could not deliver.

    Only what this run created, and only while nothing is behind it: a tag with
    a release attached belongs to that release, and removing it would leave the
    release pointing at nothing -- worse than the problem being fixed.

    Cleanup never raises. Whatever failed the release is the interesting error,
    and it must not be replaced by an error about tidying up.
    """
    if release_exists(runner, tag):
        return
    for argv in (
        ("git", "push", "origin", f":refs/tags/{tag}"),
        ("git", "tag", "-d", tag),
    ):
        try:
            runner.run(argv)
        except (OSError, subprocess.CalledProcessError):
            continue


def _repository_release_exists(runner: TagRunner, repository: str, tag: str) -> bool:
    owner, separator, name = repository.partition("/")
    if not separator or not owner or not name or "/" in name:
        raise ValueError(f"repository must be owner/name, got {repository!r}")
    query = (
        "query($owner:String!,$name:String!,$tag:String!){"
        "repository(owner:$owner,name:$name){release(tagName:$tag){tagName}}}"
    )
    raw = _capture(
        runner,
        "gh",
        "api",
        "graphql",
        "-f",
        f"query={query}",
        "-F",
        f"owner={owner}",
        "-F",
        f"name={name}",
        "-F",
        f"tag={tag}",
    )
    try:
        release = json.loads(raw)["data"]["repository"]["release"]
    except (KeyError, TypeError, json.JSONDecodeError) as exc:
        raise RuntimeError("GitHub returned malformed release identity data") from exc
    return release is not None


def discard_exact_unpublished_claim(
    runner: TagRunner,
    *,
    tag: str,
    source_commit: str,
    repository: str,
) -> bool:
    """Delete only this failed hosted run's exact unpublished version claim."""
    target = remote_version_target(runner, tag)
    if target is None:
        return False
    if target != source_commit:
        raise RuntimeError(
            f"refusing to discard {tag}: it targets {target}, not {source_commit}"
        )
    if _repository_release_exists(runner, repository, tag):
        return False
    runner.run(("git", "push", "origin", f":refs/tags/{tag}"))
    with suppress(OSError, subprocess.CalledProcessError):
        runner.run(("git", "tag", "-d", tag))
    return True


class CommandRunner:
    def run(
        self,
        argv: Sequence[str],
        *,
        capture: bool = False,
        env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            list(argv),
            check=True,
            text=True,
            stdout=subprocess.PIPE if capture else None,
            env=env,
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    cleanup = subparsers.add_parser("cleanup-exact")
    cleanup.add_argument("--tag", required=True)
    cleanup.add_argument("--source-commit", required=True)
    cleanup.add_argument("--repository", required=True)
    args = parser.parse_args()
    if not args.tag.startswith("v"):
        parser.error("--tag must start with v")
    if len(args.source_commit) != 40 or any(
        character not in "0123456789abcdef" for character in args.source_commit
    ):
        parser.error("--source-commit must be an exact lowercase commit")
    try:
        deleted = discard_exact_unpublished_claim(
            CommandRunner(),
            tag=args.tag,
            source_commit=args.source_commit,
            repository=args.repository,
        )
    except (OSError, subprocess.CalledProcessError, RuntimeError, ValueError) as error:
        print(f"version-claim cleanup failed: {error}", file=sys.stderr)
        return 1
    print(f"{'deleted' if deleted else 'kept'} version claim {args.tag}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
