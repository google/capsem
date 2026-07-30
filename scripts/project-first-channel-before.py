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
    # Deliberately not "profiles must be non-empty". The only input this can
    # ever receive is the bootstrapped source for an absent channel, and
    # `bootstrap_first_party_channel_source` emits `profiles: {}` because a new
    # channel has none yet. Requiring them made the cold start unreachable: the
    # projection rejected the sole manifest the workflow could hand it, and
    # then would have set profiles to empty anyway. Only the shape is checked.
    profiles = source.get("profiles")
    if not isinstance(profiles, dict):
        raise ValueError("serialized source profiles must be an object")

    # The channel did not exist, so its before-state is empty of both families.
    # The packages above are inherited from the donor and are validated for
    # shape, not for existence -- once a donor is retired its URLs are dead, and
    # carrying them here would make the pairing gate fetch artifacts that no
    # longer resolve. The following binary release publishes this channel's own
    # packages and activates it.
    projected = copy.deepcopy(source)
    projected["profiles"] = {}
    projected["packages"] = []
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
