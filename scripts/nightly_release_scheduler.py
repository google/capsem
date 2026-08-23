#!/usr/bin/env python3
"""Run every selected nightly release lane before returning one verdict."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Protocol

ROOT = Path(__file__).resolve().parents[1]
SOURCE_COMMIT = re.compile(r"[0-9a-f]{40}\Z")
PROFILE = re.compile(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\Z")


class CommandRunner(Protocol):
    def run(self, argv: Sequence[str]) -> int: ...


class Runner:
    def run(self, argv: Sequence[str]) -> int:
        return subprocess.run(list(argv), cwd=ROOT, check=False).returncode


@dataclass(frozen=True)
class Lane:
    name: str
    command: tuple[str, ...]


@dataclass(frozen=True)
class LaneOutcome:
    lane: str
    status: str
    exit_code: int | None
    error: str | None = None


@dataclass(frozen=True)
class ScheduleResult:
    channel: str
    source_commit: str
    lanes: tuple[LaneOutcome, ...]

    @property
    def ok(self) -> bool:
        return all(outcome.status == "success" for outcome in self.lanes)

    def as_dict(self) -> dict[str, object]:
        return {
            "event": "nightly_release_complete",
            "channel": self.channel,
            "source_commit": self.source_commit,
            "status": "success" if self.ok else "failed",
            "lanes": [asdict(outcome) for outcome in self.lanes],
        }


def schedule(channel: str, profiles: Sequence[str], source_commit: str) -> tuple[Lane, ...]:
    if channel != "nightly":
        raise ValueError("the nightly scheduler only accepts channel nightly")
    if SOURCE_COMMIT.fullmatch(source_commit) is None:
        raise ValueError("source commit must be 40-character lowercase hexadecimal")
    if not profiles:
        raise ValueError("at least one nightly profile is required")
    if len(set(profiles)) != len(profiles):
        raise ValueError("nightly profiles must be unique")
    invalid = [profile for profile in profiles if PROFILE.fullmatch(profile) is None]
    if invalid:
        raise ValueError(f"invalid nightly profiles: {invalid}")
    lanes = [
        Lane(
            f"profile/{profile}",
            ("just", "release-profile", channel, profile, source_commit),
        )
        for profile in profiles
    ]
    lanes.append(
        Lane(
            "binaries",
            ("just", "release-binaries", channel, source_commit),
        )
    )
    return tuple(lanes)


def run_schedule(
    channel: str,
    profiles: Sequence[str],
    source_commit: str,
    runner: CommandRunner,
) -> ScheduleResult:
    outcomes: list[LaneOutcome] = []
    for lane in schedule(channel, profiles, source_commit):
        print(
            json.dumps(
                {
                    "event": "nightly_lane_started",
                    "lane": lane.name,
                    "source_commit": source_commit,
                },
                sort_keys=True,
            ),
            flush=True,
        )
        try:
            exit_code = runner.run(lane.command)
        except OSError as error:
            outcome = LaneOutcome(lane.name, "launch-error", None, str(error))
        else:
            outcome = LaneOutcome(
                lane.name,
                "success" if exit_code == 0 else "failed",
                exit_code,
            )
        outcomes.append(outcome)
        print(
            json.dumps(
                {"event": "nightly_lane_complete", **asdict(outcome)},
                sort_keys=True,
            ),
            flush=True,
        )
    return ScheduleResult(channel, source_commit, tuple(outcomes))


def main(argv: Sequence[str] | None = None, *, runner: CommandRunner | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--profile", action="append", dest="profiles", required=True)
    parser.add_argument("--source-commit", required=True)
    args = parser.parse_args(argv)
    try:
        result = run_schedule(
            args.channel,
            args.profiles,
            args.source_commit,
            runner or Runner(),
        )
    except ValueError as error:
        print(f"nightly release scheduling failed: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result.as_dict(), indent=2, sort_keys=True), flush=True)
    return 0 if result.ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
