#!/usr/bin/env python3
"""Preserve live binary release metadata while publishing VM asset changes."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


def _load_url(url: str) -> dict[str, Any]:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme == "file":
        return json.loads(Path(urllib.request.url2pathname(parsed.path)).read_text())
    if parsed.scheme in {"http", "https"}:
        with urllib.request.urlopen(url, timeout=60) as response:
            return json.loads(response.read().decode("utf-8"))
    raise SystemExit(f"manifest URL must use file://, http://, or https://: {url}")


def _load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_manifest(path: Path, manifest: dict[str, Any]) -> None:
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _binary_metadata(manifest: dict[str, Any]) -> dict[str, Any] | None:
    binaries = manifest.get("binaries")
    if not isinstance(binaries, dict):
        return None
    current = binaries.get("current")
    releases = binaries.get("releases")
    if not isinstance(current, str) or not current:
        return None
    if not isinstance(releases, dict) or current not in releases:
        return None
    return binaries


def preserve_binary_metadata(
    generated_manifest: dict[str, Any],
    previous_manifest: dict[str, Any] | None,
) -> tuple[dict[str, Any], bool]:
    if previous_manifest is None:
        return generated_manifest, False
    binaries = _binary_metadata(previous_manifest)
    if binaries is None:
        return generated_manifest, False
    generated_manifest["binaries"] = binaries
    return generated_manifest, True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest-path", required=True, type=Path)
    parser.add_argument("--previous-manifest-url", required=True)
    parser.add_argument("--allow-missing-previous", action="store_true")
    args = parser.parse_args()

    try:
        previous_manifest = _load_url(args.previous_manifest_url)
    except Exception as exc:
        if not args.allow_missing_previous:
            raise SystemExit(f"could not read previous manifest: {exc}") from exc
        print(f"warning: could not read previous manifest: {exc}", file=sys.stderr)
        previous_manifest = None

    manifest = _load_manifest(args.manifest_path)
    merged, preserved = preserve_binary_metadata(manifest, previous_manifest)
    if preserved:
        _write_manifest(args.manifest_path, merged)
    print(
        json.dumps(
            {
                "manifest": str(args.manifest_path),
                "binary_metadata_preserved": preserved,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
