"""Decide whether one runtime revision still has to be released into a channel.

A runtime revision names one source commit's VM assets, so the channel's
serialized source already carrying it means this exact commit's runtime was
authored. Activating a staged runtime is the binary lane's job, not this one's.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any


def runtime_release_delta(
    source_manifest: dict[str, Any], channel: str, runtime_revision: str
) -> dict[str, Any]:
    if source_manifest.get("channel") != channel:
        raise ValueError(
            f"source manifest declares channel {source_manifest.get('channel')!r}, "
            f"expected {channel!r}"
        )
    if not runtime_revision:
        raise ValueError("runtime revision must not be empty")
    source = source_manifest.get("runtime")
    if source is not None and not isinstance(source, dict):
        raise ValueError("source manifest runtime must be an object")
    source_revision = source.get("revision") if source is not None else None
    if source is not None and (not isinstance(source_revision, str) or not source_revision):
        raise ValueError("source manifest runtime has no revision")
    if source is None:
        reason = "new_runtime"
    elif source_revision != runtime_revision:
        reason = "runtime_changed"
    else:
        reason = "already_authored"
    return {
        "schema": "capsem.runtime_release_delta.v1",
        "channel": channel,
        "runtime_revision": runtime_revision,
        "source_revision": source_revision,
        "release_needed": reason != "already_authored",
        "reason": reason,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--runtime-revision", required=True)
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args()
    try:
        source = json.loads(args.source_manifest.read_text(encoding="utf-8"))
        if not isinstance(source, dict):
            raise ValueError("source manifest must be a JSON object")
        result = runtime_release_delta(source, args.channel, args.runtime_revision)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"runtime release delta failed: {error}", file=sys.stderr)
        return 1
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"release_needed={'true' if result['release_needed'] else 'false'}\n")
            handle.write(f"reason={result['reason']}\n")
    if args.json_output is not None:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
