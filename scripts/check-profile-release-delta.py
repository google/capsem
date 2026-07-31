#!/usr/bin/env python3
"""Compare exactly one candidate channel/profile with its serialized source."""

from __future__ import annotations

import argparse
import json
import os
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any


def selected_profile_delta(
    source_manifest: dict[str, Any],
    candidate_manifest: dict[str, Any],
    channel: str,
    profile: str,
    *,
    public_manifest: dict[str, Any] | None = None,
) -> dict[str, Any]:
    manifests = [
        ("source", source_manifest),
        ("candidate", candidate_manifest),
    ]
    if public_manifest is not None:
        manifests.append(("public", public_manifest))
    for label, manifest in manifests:
        if manifest.get("channel") != channel:
            raise ValueError(
                f"{label} manifest declares channel {manifest.get('channel')!r}, "
                f"expected {channel!r}"
            )
        if not isinstance(manifest.get("profiles"), dict):
            raise ValueError(f"{label} manifest profiles must be an object")
    if not isinstance(source_manifest.get("packages"), list):
        raise ValueError("source manifest packages must be an array")
    if public_manifest is not None:
        if not isinstance(public_manifest.get("packages"), list):
            raise ValueError("public manifest packages must be an array")
        if source_manifest["packages"] != public_manifest["packages"]:
            raise ValueError(
                "serialized source package cohort differs from the public package "
                "cohort; the binary release must finish before profile activation"
            )
    candidate = candidate_manifest["profiles"].get(profile)
    if not isinstance(candidate, dict):
        raise ValueError(f"candidate manifest does not contain profile {profile!r}")
    source = source_manifest["profiles"].get(profile)
    if source is not None and not isinstance(source, dict):
        raise ValueError(f"source manifest profile {profile!r} is malformed")

    def semantic_profile(value: dict[str, Any] | None) -> dict[str, Any] | None:
        if value is None:
            return None
        normalized = deepcopy(value)
        for architecture in normalized.get("architectures", []):
            if not isinstance(architecture, dict):
                continue
            for section in ("config", "images", "evidence"):
                for row in architecture.get(section, []):
                    if isinstance(row, dict):
                        row.pop("url", None)
        return normalized

    source_changed = semantic_profile(source) != semantic_profile(candidate)
    public = (
        public_manifest["profiles"].get(profile)
        if public_manifest is not None
        else source
    )
    if public is not None and not isinstance(public, dict):
        raise ValueError(f"public manifest profile {profile!r} is malformed")
    activation_needed = semantic_profile(public) != semantic_profile(candidate)
    release_needed = source_changed or activation_needed
    if source_changed:
        reason = "new_profile" if source is None else "profile_changed"
    elif activation_needed:
        reason = "public_activation_pending"
    elif public_manifest is not None:
        reason = "already_public"
    else:
        reason = "profile_unchanged"
    return {
        "schema": "capsem.profile_release_delta.v1",
        "channel": channel,
        "profile": profile,
        "changed": release_needed,
        "source_changed": source_changed,
        "activation_needed": activation_needed,
        "release_needed": release_needed,
        "reason": reason,
        "source_revision": source.get("revision") if source else None,
        "candidate_revision": candidate.get("revision"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--candidate-manifest", type=Path, required=True)
    parser.add_argument("--public-manifest", type=Path, required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args()
    try:
        result = selected_profile_delta(
            json.loads(args.source_manifest.read_text(encoding="utf-8")),
            json.loads(args.candidate_manifest.read_text(encoding="utf-8")),
            args.channel,
            args.profile,
            public_manifest=json.loads(
                args.public_manifest.read_text(encoding="utf-8")
            ),
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"profile release delta failed: {error}", file=sys.stderr)
        return 1
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as handle:
            for field in ("source_changed", "activation_needed", "release_needed"):
                value = "true" if result[field] else "false"
                handle.write(f"{field}={value}\n")
            handle.write(f"reason={result['reason']}\n")
    if args.json_output is not None:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
