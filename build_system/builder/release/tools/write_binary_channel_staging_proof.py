"""Prove that binary staging left the selected VM asset graph unchanged."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def _read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected JSON object: {path}")
    return value


def _summary(values: set[str], *, empty: str) -> str:
    ordered = sorted(values)
    if not ordered:
        return empty
    if len(ordered) == 1:
        return ordered[0]
    return "mixed"


def write_proof(root: Path) -> Path:
    before = _read_json(root / "manifest.before.json")
    after = _read_json(root / "manifest.json")
    if "packages" in before and "packages" in after:
        if after.get("runtime") != before.get("runtime"):
            raise ValueError("binary dry-run changed runtime image metadata")
        binary_version = _summary(
            {
                package["version"]
                for package in after.get("packages", [])
                if isinstance(package, dict) and isinstance(package.get("version"), str)
            },
            empty="not_published",
        )
        runtime = after.get("runtime")
        if runtime is not None and not isinstance(runtime, dict):
            raise ValueError("binary dry-run runtime must be an object")
        revision = runtime.get("revision") if runtime is not None else None
        asset_version = revision if isinstance(revision, str) else "not_published"

    else:
        if after["assets"] != before["assets"]:
            raise ValueError("binary dry-run changed VM asset metadata")
        binary_version = after["binaries"]["current"]
        asset_version = after["assets"]["current"]

    proof = {
        "schema": "capsem.binary_channel_dry_run.v1",
        "vm_asset_jobs": "not_run",
        "vm_assets_unchanged": True,
        "binary_version": binary_version,
        "asset_version": asset_version,
        "manifest_before": "manifest.before.json",
        "manifest_after": "manifest.json",
        "record_binary_report": "record-binary.json",
    }
    output = root / "proof.json"
    output.write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    print(write_proof(args.root))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
