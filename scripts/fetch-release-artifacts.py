#!/usr/bin/env python3
"""Fetch and verify immutable package or profile inputs from a release manifest."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlparse
from urllib.request import Request, url2pathname, urlopen

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_inputs import (  # noqa: E402
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


def _assert_public_channel_absent(
    public_manifest_url: str, bootstrap_manifest: dict[str, Any]
) -> None:
    channel = bootstrap_manifest.get("channel")
    if not isinstance(channel, str) or not channel:
        raise ValueError("bootstrap manifest is missing its channel")
    parsed = urlparse(public_manifest_url)
    expected_path = f"/assets/{channel}/manifest.json"
    if parsed.scheme not in {"http", "https"} or parsed.path != expected_path:
        raise ValueError(
            "bootstrap fallback requires the selected public channel manifest URL"
        )
    catalog_url = f"{parsed.scheme}://{parsed.netloc}/channels.json"
    catalog = json.loads(_read_url(catalog_url))
    channels = catalog.get("channels") if isinstance(catalog, dict) else None
    if not isinstance(channels, dict):
        raise ValueError("public channel catalog must contain a channels object")
    if channel in channels:
        raise ValueError(
            f"public channel {channel} exists but its manifest could not be resolved"
        )


def _local_publication_payload(
    url: str,
    *,
    publication_base: str | None,
    publication_dir: Path | None,
) -> tuple[bytes, Path] | None:
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
    return path.read_bytes(), path


def fetch_release_inputs(
    manifest_url: str,
    kind: str,
    output: Path,
    *,
    local_publication_base: str | None = None,
    local_publication_dir: Path | None = None,
    allow_empty_profiles: bool = False,
    bootstrap_manifest_url: str | None = None,
) -> dict[str, Any]:
    if (local_publication_base is None) != (local_publication_dir is None):
        raise ValueError("local publication base and directory must be supplied together")
    if local_publication_base is not None and kind != "profiles":
        raise ValueError("local publication overrides are profile-only")
    if local_publication_base is not None:
        parsed_base = urlparse(local_publication_base)
        if parsed_base.scheme != "https" or not parsed_base.netloc:
            raise ValueError("local publication base must be an absolute HTTPS URL")

    try:
        manifest_bytes, manifest = _read_manifest(manifest_url)
    except (OSError, ValueError):
        if bootstrap_manifest_url is None:
            raise
        manifest_bytes, manifest = _read_manifest(bootstrap_manifest_url)
        if manifest.get("profiles") != {}:
            raise ValueError(
                "bootstrap release-input fallback requires explicit empty profiles"
            )
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
    )
    local_paths: set[Path] = set()
    for row in rows:
        local = _local_publication_payload(
            row["url"],
            publication_base=local_publication_base,
            publication_dir=local_publication_dir,
        )
        if local is None:
            payload = _read_url(row["url"])
        else:
            payload, local_path = local
            local_paths.add(local_path)
        verify_payload(payload, row["record"], row["label"])
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
        actual = {path.resolve() for path in local_publication_dir.iterdir() if path.is_file()}
        expected = local_paths | {source.resolve()}
        if actual != expected:
            raise ValueError(
                "local publication file set mismatch: "
                f"extra={sorted(str(path) for path in actual - expected)}, "
                f"missing={sorted(str(path) for path in expected - actual)}"
            )
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
    parser.add_argument("--bootstrap-manifest-url")
    args = parser.parse_args()
    try:
        report = fetch_release_inputs(
            args.manifest_url,
            args.kind,
            args.output,
            local_publication_base=args.local_publication_base,
            local_publication_dir=args.local_publication_dir,
            allow_empty_profiles=args.allow_empty_profiles,
            bootstrap_manifest_url=args.bootstrap_manifest_url,
        )
    except (OSError, ValueError) as error:
        print(f"release input fetch failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
