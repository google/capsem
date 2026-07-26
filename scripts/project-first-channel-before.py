#!/usr/bin/env python3
"""Project the exact inactive state before a first channel profile activation."""

from __future__ import annotations

import argparse
import copy
import json
import sys
from pathlib import Path
from typing import Any


def project_first_channel_before(
    source: dict[str, Any],
    *,
    channel: str,
    bootstrap: bool,
) -> dict[str, Any]:
    """Return a test-only pre-profile projection of a serialized channel source."""
    if bootstrap is not True:
        raise ValueError("first-channel projection requires an absent public channel")
    if source.get("channel") != channel:
        raise ValueError(
            f"serialized source declares channel {source.get('channel')!r}, "
            f"expected {channel!r}"
        )
    if source.get("status") != "current":
        raise ValueError("serialized source must be the current channel source")
    packages = source.get("packages")
    if not isinstance(packages, list) or not packages:
        raise ValueError("serialized source must contain an official package cohort")
    if any(
        not isinstance(package, dict) or package.get("status") != "current"
        for package in packages
    ):
        raise ValueError("serialized source package cohort must be entirely current")
    profiles = source.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("serialized source must contain non-empty profiles")

    projected = copy.deepcopy(source)
    projected["profiles"] = {}
    return projected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--bootstrap", required=True, choices=("true", "false"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        source = json.loads(args.source_manifest.read_text(encoding="utf-8"))
        if not isinstance(source, dict):
            raise ValueError("serialized source manifest must be a JSON object")
        projected = project_first_channel_before(
            source,
            channel=args.channel,
            bootstrap=args.bootstrap == "true",
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(projected, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"first-channel before projection failed: {error}", file=sys.stderr)
        return 1
    print(f"projected inactive first-channel state at {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
