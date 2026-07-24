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
) -> dict[str, Any]:
    if (local_publication_base is None) != (local_publication_dir is None):
        raise ValueError("local publication base and directory must be supplied together")
    if local_publication_base is not None and kind != "profiles":
        raise ValueError("local publication overrides are profile-only")
    if local_publication_base is not None:
        parsed_base = urlparse(local_publication_base)
        if parsed_base.scheme != "https" or not parsed_base.netloc:
            raise ValueError("local publication base must be an absolute HTTPS URL")

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

    rows = resolved_artifact_rows(manifest, manifest_url, kind)
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
    args = parser.parse_args()
    try:
        report = fetch_release_inputs(
            args.manifest_url,
            args.kind,
            args.output,
            local_publication_base=args.local_publication_base,
            local_publication_dir=args.local_publication_dir,
        )
    except (OSError, ValueError) as error:
        print(f"release input fetch failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
