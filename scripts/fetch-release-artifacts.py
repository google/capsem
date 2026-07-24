#!/usr/bin/env python3
"""Fetch and verify immutable package or profile inputs from a release manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path, PurePosixPath
from typing import Any, Iterable
from urllib.parse import unquote, urljoin, urlparse
from urllib.request import Request, url2pathname, urlopen

import blake3


USER_AGENT = "capsem-release-artifact-fetcher/1"


def _read_url(url: str) -> bytes:
    parsed = urlparse(url)
    if parsed.scheme == "file":
        return Path(url2pathname(parsed.path)).read_bytes()
    if parsed.scheme not in {"http", "https"}:
        raise ValueError(f"release input must use file://, http://, or https://: {url}")
    request = Request(url, headers={"User-Agent": USER_AGENT})
    with urlopen(request, timeout=120) as response:
        return response.read()


def _safe_name(url: str, fallback: str) -> str:
    value = unquote(PurePosixPath(urlparse(url).path).name) or fallback
    if value in {".", ".."} or "/" in value or "\\" in value:
        raise ValueError(f"unsafe release artifact name: {value!r}")
    return value


def _required_digest(record: dict[str, Any], label: str) -> tuple[str, str, int]:
    digest = record.get("digest")
    if not isinstance(digest, dict):
        raise ValueError(f"{label} has no digest")
    sha256 = digest.get("sha256")
    b3 = digest.get("blake3")
    size = record.get("bytes")
    if not isinstance(sha256, str) or len(sha256) != 64:
        raise ValueError(f"{label} has invalid SHA-256")
    if not isinstance(b3, str) or len(b3) != 64:
        raise ValueError(f"{label} has invalid BLAKE3")
    if not isinstance(size, int) or size < 0:
        raise ValueError(f"{label} has invalid byte size")
    return sha256, b3, size


def _verify(payload: bytes, record: dict[str, Any], label: str) -> None:
    sha256, b3, size = _required_digest(record, label)
    if len(payload) != size:
        raise ValueError(f"{label} byte size mismatch: {len(payload)} != {size}")
    actual_sha256 = hashlib.sha256(payload).hexdigest()
    if actual_sha256 != sha256:
        raise ValueError(f"{label} SHA-256 mismatch")
    actual_b3 = blake3.blake3(payload).hexdigest()
    if actual_b3 != b3:
        raise ValueError(f"{label} BLAKE3 mismatch")


def _package_rows(
    manifest: dict[str, Any], manifest_url: str
) -> Iterable[tuple[Path, str, dict[str, Any], str]]:
    packages = manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        raise ValueError("release manifest contains no packages")
    selected = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("status") == "current"
    ]
    if not selected:
        raise ValueError("release manifest contains no current packages")
    for index, package in enumerate(selected):
        url = package.get("url")
        if not isinstance(url, str) or not url:
            raise ValueError(f"package[{index}] has no URL")
        absolute = urljoin(manifest_url, url)
        name = _safe_name(absolute, f"package-{index}")
        yield Path(name), absolute, package, f"package {name}"


def _profile_rows(
    manifest: dict[str, Any], manifest_url: str
) -> Iterable[tuple[Path, str, dict[str, Any], str]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release manifest contains no profiles")
    seen: dict[str, tuple[str, str, int]] = {}
    for profile_id, profile in sorted(profiles.items()):
        if not isinstance(profile_id, str) or not isinstance(profile, dict):
            raise ValueError("release manifest profile rows are malformed")
        architectures = profile.get("architectures")
        if not isinstance(architectures, list) or not architectures:
            raise ValueError(f"profile {profile_id} has no architectures")
        for architecture in architectures:
            if not isinstance(architecture, dict):
                raise ValueError(f"profile {profile_id} architecture is malformed")
            arch = architecture.get("architecture")
            if not isinstance(arch, str) or not arch:
                raise ValueError(f"profile {profile_id} architecture has no name")
            for section in ("config", "images", "evidence"):
                records = architecture.get(section, [])
                if not isinstance(records, list):
                    raise ValueError(f"profile {profile_id}/{arch} {section} is malformed")
                for index, record in enumerate(records):
                    if not isinstance(record, dict):
                        raise ValueError(
                            f"profile {profile_id}/{arch} {section}[{index}] is malformed"
                        )
                    url = record.get("url")
                    if not isinstance(url, str) or not url:
                        raise ValueError(
                            f"profile {profile_id}/{arch} {section}[{index}] has no URL"
                        )
                    absolute = urljoin(manifest_url, url)
                    label = str(
                        record.get("name")
                        or record.get("path")
                        or record.get("kind")
                        or f"{section}-{index}"
                    )
                    digest = _required_digest(
                        record, f"profile {profile_id}/{arch}/{label}"
                    )
                    previous = seen.setdefault(absolute, digest)
                    if previous != digest:
                        raise ValueError(
                            f"profile records disagree on immutable input {absolute}"
                        )
                    name = _safe_name(absolute, f"{section}-{index}")
                    yield (
                        Path("profiles") / profile_id / arch / section / name,
                        absolute,
                        record,
                        f"profile {profile_id}/{arch}/{label}",
                    )


def fetch_release_inputs(
    manifest_url: str, kind: str, output: Path
) -> dict[str, Any]:
    manifest_bytes = _read_url(manifest_url)
    try:
        manifest = json.loads(manifest_bytes)
    except json.JSONDecodeError as error:
        raise ValueError(f"release manifest is invalid JSON: {error}") from error
    if not isinstance(manifest, dict):
        raise ValueError("release manifest must contain a JSON object")

    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    (output / "manifest.json").write_bytes(manifest_bytes)

    rows = _package_rows(manifest, manifest_url) if kind == "packages" else _profile_rows(
        manifest, manifest_url
    )
    fetched: list[dict[str, Any]] = []
    written_urls: set[str] = set()
    for relative, url, record, label in rows:
        if url in written_urls:
            continue
        payload = _read_url(url)
        _verify(payload, record, label)
        path = output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        written_urls.add(url)
        fetched.append(
            {
                "path": str(relative),
                "url": url,
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3.blake3(payload).hexdigest(),
            }
        )
    if not fetched:
        raise ValueError(f"release manifest resolved no {kind}")
    report = {
        "schema": "capsem.release_inputs.v1",
        "kind": kind,
        "manifest_url": manifest_url,
        "output": str(output),
        "artifacts": fetched,
    }
    (output / "release-inputs.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-url", required=True)
    parser.add_argument("--kind", choices=("packages", "profiles"), required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        report = fetch_release_inputs(args.manifest_url, args.kind, args.output)
    except (OSError, ValueError) as error:
        print(f"release input fetch failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
