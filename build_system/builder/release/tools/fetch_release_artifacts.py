"""Fetch and verify immutable package or profile inputs from a release manifest."""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlparse
from urllib.request import Request, url2pathname, urlopen

from .release_inputs import (
    report_artifacts,
    resolved_artifact_rows,
    verify_payload,
)

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


def _read_manifest(url: str) -> tuple[bytes, dict[str, Any]]:
    manifest_bytes = _read_url(url)
    try:
        manifest = json.loads(manifest_bytes)
    except json.JSONDecodeError as error:
        raise ValueError(f"release manifest is invalid JSON: {error}") from error
    if not isinstance(manifest, dict):
        raise ValueError("release manifest must contain a JSON object")
    return manifest_bytes, manifest


def _cache_path(cache_dir: Path, row: dict[str, Any]) -> Path:
    digest = row["sha256"]
    return cache_dir / "sha256" / digest[:2] / digest


def _cached_payload(cache_dir: Path, row: dict[str, Any]) -> bytes | None:
    path = _cache_path(cache_dir, row)
    if not path.exists():
        return None
    if not path.is_file():
        raise ValueError(f"release input cache entry is not a file: {path}")
    payload = path.read_bytes()
    try:
        verify_payload(payload, row["record"], f"cached {row['label']}")
    except ValueError:
        path.unlink()
        return None
    return payload


def _store_cached_payload(cache_dir: Path, row: dict[str, Any], payload: bytes) -> None:
    path = _cache_path(cache_dir, row)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_bytes(payload)
    os.replace(temporary, path)


def _prune_cache(cache_dir: Path, keep_sha256: set[str]) -> None:
    root = cache_dir / "sha256"
    if not root.exists():
        return
    for prefix in root.iterdir():
        if (
            not prefix.is_dir()
            or len(prefix.name) != 2
            or any(character not in "0123456789abcdef" for character in prefix.name)
        ):
            raise ValueError(f"unexpected release input cache entry: {prefix}")
        for entry in prefix.iterdir():
            if (
                not entry.is_file()
                or len(entry.name) != 64
                or any(character not in "0123456789abcdef" for character in entry.name)
            ):
                raise ValueError(f"unexpected release input cache entry: {entry}")
            if entry.name not in keep_sha256:
                entry.unlink()
        with contextlib.suppress(OSError):
            prefix.rmdir()


def _assert_public_channel_absent(
    public_manifest_url: str, bootstrap_manifest: dict[str, Any]
) -> None:
    channel = bootstrap_manifest.get("channel")
    if not isinstance(channel, str) or not channel:
        raise ValueError("bootstrap manifest is missing its channel")
    parsed = urlparse(public_manifest_url)
    expected_path = f"/assets/{channel}/manifest.json"
    if parsed.scheme not in {"http", "https"} or parsed.path != expected_path:
        raise ValueError("bootstrap fallback requires the selected public channel manifest URL")
    catalog_url = f"{parsed.scheme}://{parsed.netloc}/channels.json"
    catalog = json.loads(_read_url(catalog_url))
    channels = catalog.get("channels") if isinstance(catalog, dict) else None
    if not isinstance(channels, dict):
        raise ValueError("public channel catalog must contain a channels object")
    if channel in channels:
        raise ValueError(f"public channel {channel} exists but its manifest could not be resolved")


def _local_publication_path(
    url: str,
    *,
    publication_base: str | None,
    publication_dir: Path | None,
) -> Path | None:
    if publication_base is None or publication_dir is None:
        return None
    prefix = publication_base.rstrip("/") + "/"
    if not url.startswith(prefix):
        return None
    relative = url.removeprefix(prefix)
    if not relative or "/" in relative or "\\" in relative or Path(relative).name != relative:
        raise ValueError(f"local publication URL has an unsafe artifact name: {url}")
    root = publication_dir.resolve()
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError as error:
        raise ValueError(f"local publication artifact escapes {root}: {url}") from error
    if not path.is_file():
        raise ValueError(f"local publication artifact is missing: {path}")
    return path


def _local_publication_payload(
    url: str,
    *,
    publication_base: str | None,
    publication_dir: Path | None,
) -> tuple[bytes, Path] | None:
    path = _local_publication_path(
        url,
        publication_base=publication_base,
        publication_dir=publication_dir,
    )
    if path is None:
        return None
    return path.read_bytes(), path


def fetch_release_inputs(
    manifest_url: str,
    kind: str,
    output: Path,
    *,
    local_publication_base: str | None = None,
    local_publication_dir: Path | None = None,
    allow_empty_profiles: bool = False,
    allow_empty_packages: bool = False,
    bootstrap_manifest_url: str | None = None,
    architecture: str | None = None,
    cache_dir: Path | None = None,
    prune_cache: bool = False,
) -> dict[str, Any]:
    if (local_publication_base is None) != (local_publication_dir is None):
        raise ValueError("local publication base and directory must be supplied together")
    if local_publication_base is not None and kind != "profiles":
        raise ValueError("local publication overrides are profile-only")
    if local_publication_base is not None:
        parsed_base = urlparse(local_publication_base)
        if parsed_base.scheme != "https" or not parsed_base.netloc:
            raise ValueError("local publication base must be an absolute HTTPS URL")
    if architecture is not None and kind != "profiles":
        raise ValueError("architecture filtering is profile-only")
    if prune_cache and cache_dir is None:
        raise ValueError("cache pruning requires a cache directory")
    if cache_dir is not None:
        output_root = output.resolve()
        cache_root = cache_dir.resolve()
        if (
            output_root == cache_root
            or output_root in cache_root.parents
            or cache_root in output_root.parents
        ):
            raise ValueError("release input output and cache directories must be separate")

    try:
        manifest_bytes, manifest = _read_manifest(manifest_url)
    except (OSError, ValueError):
        if bootstrap_manifest_url is None:
            raise
        manifest_bytes, manifest = _read_manifest(bootstrap_manifest_url)
        if manifest.get("profiles") != {}:
            raise ValueError(
                "bootstrap release-input fallback requires explicit empty profiles"
            ) from None
        _assert_public_channel_absent(manifest_url, manifest)
        manifest_url = bootstrap_manifest_url

    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    (output / "manifest.json").write_bytes(manifest_bytes)

    rows = resolved_artifact_rows(
        manifest,
        manifest_url,
        kind,
        allow_empty_profiles=allow_empty_profiles,
        allow_empty_packages=allow_empty_packages,
        architecture=architecture,
    )
    local_paths: set[Path] = set()
    cache_hits = 0
    cache_misses = 0
    for row in rows:
        store_in_cache = False
        local = _local_publication_payload(
            row["url"],
            publication_base=local_publication_base,
            publication_dir=local_publication_dir,
        )
        if local is None:
            payload = _cached_payload(cache_dir, row) if cache_dir is not None else None
            if payload is None:
                payload = _read_url(row["url"])
                if cache_dir is not None:
                    store_in_cache = True
            elif cache_dir is not None:
                cache_hits += 1
        else:
            payload, local_path = local
            local_paths.add(local_path)
        verify_payload(payload, row["record"], row["label"])
        if store_in_cache and cache_dir is not None:
            _store_cached_payload(cache_dir, row, payload)
            cache_misses += 1
        path = output / row["relative"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    if local_publication_dir is not None:
        if not local_paths:
            raise ValueError("local publication base does not match any manifest profile artifact")
        source = local_publication_dir.resolve() / f"channel-source-{manifest.get('channel')}.json"
        if not source.is_file() or source.read_bytes() != manifest_bytes:
            raise ValueError(
                "local publication source manifest does not match the candidate manifest"
            )
        publication_rows = resolved_artifact_rows(
            manifest,
            manifest_url,
            kind,
            allow_empty_profiles=allow_empty_profiles,
            allow_empty_packages=allow_empty_packages,
        )
        publication_paths = {
            path
            for row in publication_rows
            if (
                path := _local_publication_path(
                    row["url"],
                    publication_base=local_publication_base,
                    publication_dir=local_publication_dir,
                )
            )
            is not None
        }
        actual = {path.resolve() for path in local_publication_dir.iterdir() if path.is_file()}
        expected = publication_paths | {source.resolve()}
        if actual != expected:
            raise ValueError(
                "local publication file set mismatch: "
                f"extra={sorted(str(path) for path in actual - expected)}, "
                f"missing={sorted(str(path) for path in expected - actual)}"
            )
    if prune_cache and cache_dir is not None:
        _prune_cache(cache_dir, {row["sha256"] for row in rows})
    fetched = report_artifacts(rows)
    report = {
        "schema": "capsem.release_inputs.v1",
        "kind": kind,
        "manifest_url": manifest_url,
        "output": str(output),
        "artifacts": fetched,
    }
    if allow_empty_profiles:
        report["allow_empty_profiles"] = True
    if allow_empty_packages:
        report["allow_empty_packages"] = True
    if architecture is not None:
        report["architecture"] = architecture
    if cache_dir is not None:
        report["cache"] = {
            "hits": cache_hits,
            "misses": cache_misses,
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
    parser.add_argument("--local-publication-base")
    parser.add_argument("--local-publication-dir", type=Path)
    parser.add_argument("--allow-empty-profiles", action="store_true")
    parser.add_argument("--allow-empty-packages", action="store_true")
    parser.add_argument("--bootstrap-manifest-url")
    parser.add_argument("--architecture", choices=("arm64", "x86_64"))
    parser.add_argument("--cache-dir", type=Path)
    parser.add_argument("--prune-cache", action="store_true")
    args = parser.parse_args()
    try:
        report = fetch_release_inputs(
            args.manifest_url,
            args.kind,
            args.output,
            local_publication_base=args.local_publication_base,
            local_publication_dir=args.local_publication_dir,
            allow_empty_profiles=args.allow_empty_profiles,
            allow_empty_packages=args.allow_empty_packages,
            bootstrap_manifest_url=args.bootstrap_manifest_url,
            architecture=args.architecture,
            cache_dir=args.cache_dir,
            prune_cache=args.prune_cache,
        )
    except (OSError, ValueError) as error:
        print(f"release input fetch failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
