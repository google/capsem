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

    rows = resolved_artifact_rows(manifest, manifest_url, kind)
    for row in rows:
        payload = _read_url(row["url"])
        verify_payload(payload, row["record"], row["label"])
        path = output / row["relative"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
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
