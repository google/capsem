#!/usr/bin/env python3
"""Enforce allowed mutation sets between two release graph JSON documents."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Iterable


JsonPath = tuple[str, ...]


def changed_paths(old: Any, new: Any, prefix: JsonPath = ()) -> set[JsonPath]:
    if old == new:
        return set()
    if isinstance(old, dict) and isinstance(new, dict):
        paths: set[JsonPath] = set()
        for key in sorted(set(old) | set(new)):
            paths.update(changed_paths(old.get(key), new.get(key), (*prefix, str(key))))
        return paths
    if isinstance(old, list) and isinstance(new, list):
        paths = set()
        for index in range(max(len(old), len(new))):
            left = old[index] if index < len(old) else None
            right = new[index] if index < len(new) else None
            paths.update(changed_paths(left, right, (*prefix, str(index))))
        return paths
    return {prefix}


def allowed_path(path: JsonPath, *, lane: str, channel: str, profile: str | None) -> bool:
    if _is_channel_manifest_pointer(path, channel):
        return True
    if len(path) < 4 or path[:2] != ("manifests", channel):
        return False

    manifest_field = path[3]
    if lane == "binary":
        return manifest_field in {"version", "status", "packages", "binaries"}
    if lane == "profile":
        return profile is not None and path[:5] == (
            "manifests",
            channel,
            path[2],
            "profiles",
            profile,
        )
    if lane == "channel":
        return manifest_field in {"version", "status"}
    raise ValueError(f"unknown release graph diff lane: {lane}")


def violations(
    old: dict[str, Any],
    new: dict[str, Any],
    *,
    lane: str,
    channel: str,
    profile: str | None,
) -> list[str]:
    paths = changed_paths(old, new)
    return [
        ".".join(path)
        for path in sorted(paths)
        if not allowed_path(path, lane=lane, channel=channel, profile=profile)
    ]


def diff_summary(
    old: dict[str, Any],
    new: dict[str, Any],
    *,
    lane: str,
    channel: str,
    profile: str | None,
) -> dict[str, Any]:
    paths = sorted(changed_paths(old, new))
    allowed: list[str] = []
    blocked: list[str] = []
    for path in paths:
        rendered = ".".join(path)
        if allowed_path(path, lane=lane, channel=channel, profile=profile):
            allowed.append(rendered)
        else:
            blocked.append(rendered)
    return {
        "lane": lane,
        "channel": channel,
        "profile": profile,
        "accepted": not blocked,
        "changed_paths": [".".join(path) for path in paths],
        "allowed_paths": allowed,
        "violations": blocked,
    }


def _is_channel_manifest_pointer(path: JsonPath, channel: str) -> bool:
    return len(path) >= 3 and path[:3] == ("channels", channel, "manifests")


def _load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"{path} must contain a JSON object")
    return value


def _write_lines(lines: Iterable[str]) -> None:
    for line in lines:
        print(line, file=sys.stderr)


def _write_summary(path: Path, summary: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--old", required=True, type=Path)
    parser.add_argument("--new", required=True, type=Path)
    parser.add_argument("--lane", required=True, choices=["binary", "profile", "channel"])
    parser.add_argument("--channel", required=True)
    parser.add_argument("--profile")
    parser.add_argument("--summary", type=Path)
    args = parser.parse_args()

    if args.lane == "profile" and not args.profile:
        raise SystemExit("--profile is required for profile lane diff checks")

    old = _load_json(args.old)
    new = _load_json(args.new)
    summary = diff_summary(
        old,
        new,
        lane=args.lane,
        channel=args.channel,
        profile=args.profile,
    )
    if args.summary:
        _write_summary(args.summary, summary)
    blocked = summary["violations"]
    if blocked:
        _write_lines(["release graph diff rejected:"] + [f"- {path}" for path in blocked])
        return 1
    print("release graph diff accepted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
