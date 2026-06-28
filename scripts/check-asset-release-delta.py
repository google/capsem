#!/usr/bin/env python3
"""Compare a newly generated asset manifest against the current channel manifest."""

from __future__ import annotations

import argparse
import json
import os
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


def _current_asset_fingerprint(manifest: dict[str, Any]) -> dict[str, Any]:
    current = manifest.get("assets", {}).get("current")
    releases = manifest.get("assets", {}).get("releases", {})
    if not isinstance(current, str) or not current:
        raise SystemExit("manifest missing assets.current")
    release = releases.get(current)
    if not isinstance(release, dict):
        raise SystemExit(f"manifest missing assets.releases[{current!r}]")
    arches = release.get("arches")
    if not isinstance(arches, dict) or not arches:
        raise SystemExit(f"manifest assets.releases[{current!r}].arches is empty")

    files: dict[str, dict[str, Any]] = {}
    for arch, assets in sorted(arches.items()):
        if not isinstance(assets, dict):
            raise SystemExit(f"manifest arch {arch!r} assets must be an object")
        for logical_name, entry in sorted(assets.items()):
            if not isinstance(entry, dict):
                raise SystemExit(f"manifest asset {arch}/{logical_name} must be an object")
            digest = entry.get("hash")
            size = entry.get("size")
            if not isinstance(digest, str) or len(digest) != 64:
                raise SystemExit(f"manifest asset {arch}/{logical_name} has invalid hash")
            if not isinstance(size, int) or size < 0:
                raise SystemExit(f"manifest asset {arch}/{logical_name} has invalid size")
            files[f"{arch}/{logical_name}"] = {"hash": digest, "size": size}
    return {"version": current, "files": files}


def _release_metadata(release: dict[str, Any]) -> dict[str, Any]:
    arches = release.get("arches", {})
    arch_names = sorted(arches.keys()) if isinstance(arches, dict) else []
    return {
        "date": release.get("date"),
        "deprecated": release.get("deprecated", False),
        "deprecated_date": release.get("deprecated_date"),
        "min_binary": release.get("min_binary"),
        "arches": arch_names,
    }


def _asset_release_metadata(manifest: dict[str, Any]) -> dict[str, Any]:
    current = manifest.get("assets", {}).get("current")
    releases = manifest.get("assets", {}).get("releases", {})
    if not isinstance(current, str) or not current:
        raise SystemExit("manifest missing assets.current")
    if not isinstance(releases, dict):
        raise SystemExit("manifest assets.releases must be an object")
    current_release = releases.get(current)
    if not isinstance(current_release, dict):
        raise SystemExit(f"manifest missing assets.releases[{current!r}]")

    historical: dict[str, Any] = {}
    for version, release in sorted(releases.items()):
        if version == current:
            continue
        if not isinstance(release, dict):
            raise SystemExit(f"manifest assets.releases[{version!r}] must be an object")
        historical[version] = _release_metadata(release)
    return {
        "current": _release_metadata(current_release),
        "historical": historical,
    }


def _write_github_output(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def _write_summary(path: str | None, result: dict[str, Any]) -> None:
    if not path:
        return
    changed = result["changed"]
    reason = result["reason"]
    with open(path, "a", encoding="utf-8") as handle:
        handle.write("## VM Asset Release Delta\n\n")
        if changed:
            if result.get("asset_blobs_changed"):
                handle.write(f"Asset publication should continue: `{reason}`.\n\n")
            else:
                handle.write(
                    f"Release-channel metadata changed: `{reason}`. The site deploy "
                    "should continue, but immutable VM blobs will not be republished.\n\n"
                )
        else:
            handle.write(
                "No VM asset changes detected; release-channel deploy will be skipped.\n\n"
            )
        handle.write(f"- Previous assets: `{result.get('previous_assets', 'unavailable')}`\n")
        handle.write(f"- New assets: `{result['new_assets']}`\n")


def _write_json_output(path: Path | None, result: dict[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def compare_manifests(
    new_manifest: dict[str, Any], previous_manifest: dict[str, Any] | None
) -> dict[str, Any]:
    new = _current_asset_fingerprint(new_manifest)
    new_metadata = _asset_release_metadata(new_manifest)
    if previous_manifest is None:
        return {
            "changed": True,
            "asset_blobs_changed": True,
            "reason": "previous_manifest_unavailable",
            "new_assets": new["version"],
        }
    previous = _current_asset_fingerprint(previous_manifest)
    previous_metadata = _asset_release_metadata(previous_manifest)
    if new["files"] != previous["files"]:
        return {
            "changed": True,
            "asset_blobs_changed": True,
            "reason": "asset_hashes_changed",
            "previous_assets": previous["version"],
            "new_assets": new["version"],
        }
    if new_metadata != previous_metadata:
        return {
            "changed": True,
            "asset_blobs_changed": False,
            "reason": "asset_release_metadata_changed",
            "previous_assets": previous["version"],
            "new_assets": new["version"],
        }
    if new["files"] == previous["files"]:
        return {
            "changed": False,
            "asset_blobs_changed": False,
            "reason": "asset_hashes_unchanged",
            "previous_assets": previous["version"],
            "new_assets": new["version"],
        }
    raise AssertionError("unreachable asset release delta state")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--new-manifest", required=True, type=Path)
    parser.add_argument("--previous-manifest-url", required=True)
    parser.add_argument("--allow-missing-previous", action="store_true")
    parser.add_argument("--summary", default=os.environ.get("GITHUB_STEP_SUMMARY"))
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args()

    new_manifest = json.loads(args.new_manifest.read_text())
    previous_manifest: dict[str, Any] | None
    try:
        previous_manifest = _load_url(args.previous_manifest_url)
    except Exception as exc:
        if not args.allow_missing_previous:
            raise SystemExit(f"could not read previous manifest: {exc}") from exc
        print(f"warning: could not read previous manifest: {exc}", file=sys.stderr)
        previous_manifest = None

    result = compare_manifests(new_manifest, previous_manifest)
    _write_github_output(
        os.environ.get("GITHUB_OUTPUT"),
        {
            "changed": "true" if result["changed"] else "false",
            "asset_blobs_changed": "true" if result["asset_blobs_changed"] else "false",
            "reason": result["reason"],
            "new_assets": result["new_assets"],
            "previous_assets": result.get("previous_assets", ""),
        },
    )
    _write_summary(args.summary, result)
    _write_json_output(args.json_output, result)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
