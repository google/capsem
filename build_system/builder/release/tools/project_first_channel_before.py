"""Project an exact empty public-before state under bootstrap authority."""

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
    retired: bool,
) -> dict[str, Any]:
    """Return the test-only public-before projection of a serialized source."""
    if bootstrap is not True:
        raise ValueError("first-channel projection requires bootstrap authority")
    if source.get("channel") != channel:
        raise ValueError(
            f"serialized source declares channel {source.get('channel')!r}, expected {channel!r}"
        )
    if source.get("status") != "current":
        raise ValueError("serialized source must be the current channel source")
    packages = source.get("packages")
    if not isinstance(packages, list):
        raise ValueError("serialized source packages must be an array")
    if retired:
        if packages:
            raise ValueError("retired serialized source must have empty package membership")
    else:
        if not packages:
            raise ValueError("serialized source must contain an official package cohort")
        if any(
            not isinstance(package, dict) or package.get("status") != "current"
            for package in packages
        ):
            raise ValueError("serialized source package cohort must be entirely current")
    # A fresh bootstrap is profileless, while a retried lane may resolve a
    # previously staged profile source. Both project to the same public-before
    # state, so only the source shape is checked here.
    profiles = source.get("profiles")
    if not isinstance(profiles, dict):
        raise ValueError("serialized source profiles must be an object")

    # A missing channel or the one digest-authorized retired graph has no usable
    # public-before family. Donor packages are validated as official authoring
    # input, then excluded from this channel-scoped projection. The ordinary
    # binary lane later supplies and activates this channel's package cohort.
    projected = copy.deepcopy(source)
    projected["profiles"] = {}
    projected["packages"] = []
    return projected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--bootstrap", required=True, choices=("true", "false"))
    parser.add_argument("--retired", required=True, choices=("true", "false"))
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
            retired=args.retired == "true",
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
