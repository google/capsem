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


PRESERVE_BINARY_USER_AGENT = "CapsemReleaseValidator/1.0"


def _load_url(url: str) -> dict[str, Any]:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme == "file":
        return json.loads(Path(urllib.request.url2pathname(parsed.path)).read_text())
    if parsed.scheme in {"http", "https"}:
        request = urllib.request.Request(
            url,
            headers={"User-Agent": PRESERVE_BINARY_USER_AGENT},
        )
        with urllib.request.urlopen(request, timeout=60) as response:
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


def _digest_fields(item: dict[str, Any]) -> tuple[str, str] | None:
    digest = item.get("digest")
    if not isinstance(digest, dict):
        return None
    sha256 = digest.get("sha256")
    blake3 = digest.get("blake3")
    if not isinstance(sha256, str) or not isinstance(blake3, str):
        return None
    return sha256, blake3


def _graph_binary_version(manifest: dict[str, Any], packages: list[Any]) -> str | None:
    versions = {
        package.get("version")
        for package in packages
        if isinstance(package, dict) and isinstance(package.get("version"), str)
    }
    if len(versions) == 1:
        return next(iter(versions))
    graph_version = manifest.get("version")
    if isinstance(graph_version, str) and "+assets." in graph_version:
        return graph_version.split("+assets.", 1)[0]
    return None


def _graph_package_file(package: dict[str, Any]) -> dict[str, Any] | None:
    digest = _digest_fields(package)
    if digest is None:
        return None
    sha256, blake3 = digest
    name = package.get("name")
    size = package.get("bytes")
    if not isinstance(name, str) or not isinstance(size, int):
        return None
    binaries = []
    for binary in package.get("binaries", []):
        if not isinstance(binary, dict):
            continue
        binary_digest = _digest_fields(binary)
        if binary_digest is None:
            continue
        binary_sha256, binary_blake3 = binary_digest
        binary_name = binary.get("name")
        installed_path = binary.get("installed_path")
        binary_size = binary.get("bytes")
        if (
            not isinstance(binary_name, str)
            or not isinstance(installed_path, str)
            or not isinstance(binary_size, int)
        ):
            continue
        binaries.append(
            {
                "name": binary_name,
                "description": binary.get("description", ""),
                "installed_path": installed_path,
                "size": binary_size,
                "sha256": binary_sha256,
                "blake3": binary_blake3,
                "sbom_component_ref": binary.get("sbom_component_ref", ""),
            }
        )
    return {
        "name": name,
        "size": size,
        "sha256": sha256,
        "blake3": blake3,
        "binaries": binaries,
    }


def _graph_evidence_file(evidence: dict[str, Any]) -> dict[str, Any] | None:
    if evidence.get("kind") != "sbom":
        return None
    digest = _digest_fields(evidence)
    if digest is None:
        return None
    sha256, blake3 = digest
    name = evidence.get("name")
    size = evidence.get("bytes")
    if not isinstance(name, str) or not isinstance(size, int):
        return None
    return {
        "name": name,
        "size": size,
        "sha256": sha256,
        "blake3": blake3,
    }


def _binary_metadata_from_graph_manifest(
    generated_manifest: dict[str, Any], previous_manifest: dict[str, Any]
) -> dict[str, Any] | None:
    packages = previous_manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        return None
    version = _graph_binary_version(previous_manifest, packages)
    if not isinstance(version, str) or not version:
        return None

    files: list[dict[str, Any]] = []
    seen_names: set[str] = set()
    for package in packages:
        if not isinstance(package, dict):
            continue
        package_file = _graph_package_file(package)
        if package_file is not None and package_file["name"] not in seen_names:
            seen_names.add(package_file["name"])
            files.append(package_file)
        for evidence in package.get("evidence", []):
            if not isinstance(evidence, dict):
                continue
            evidence_file = _graph_evidence_file(evidence)
            if evidence_file is not None and evidence_file["name"] not in seen_names:
                seen_names.add(evidence_file["name"])
                files.append(evidence_file)
    if not files:
        return None
    current_assets = generated_manifest.get("assets", {}).get("current")
    return {
        "current": version,
        "releases": {
            version: {
                "deprecated": False,
                "min_assets": current_assets if isinstance(current_assets, str) else "",
                "version": version,
                "files": files,
            }
        },
    }


def preserve_binary_metadata(
    generated_manifest: dict[str, Any],
    previous_manifest: dict[str, Any] | None,
) -> tuple[dict[str, Any], bool]:
    if previous_manifest is None:
        return generated_manifest, False
    binaries = _binary_metadata(previous_manifest)
    if binaries is None:
        binaries = _binary_metadata_from_graph_manifest(generated_manifest, previous_manifest)
    if binaries is None:
        return generated_manifest, False
    generated_manifest["binaries"] = binaries
    return generated_manifest, True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest-path", "--manifest", required=True, type=Path)
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
