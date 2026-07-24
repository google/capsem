#!/usr/bin/env python3
"""Verify a previously resolved immutable release-input directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path, PurePosixPath
from typing import Any

import blake3


def _safe_relative(value: object) -> Path:
    if not isinstance(value, str) or not value:
        raise ValueError("release input report contains an invalid path")
    relative = PurePosixPath(value)
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"release input report contains unsafe path {value!r}")
    return Path(*relative.parts)


def verify_release_inputs(input_dir: Path) -> dict[str, Any]:
    report_path = input_dir / "release-inputs.json"
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid release input report {report_path}: {error}") from error
    if not isinstance(report, dict) or report.get("schema") != "capsem.release_inputs.v1":
        raise ValueError("release input report has an unsupported schema")
    kind = report.get("kind")
    if kind not in {"packages", "profiles"}:
        raise ValueError("release input report has an invalid artifact kind")
    manifest_path = input_dir / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid resolved manifest {manifest_path}: {error}") from error
    if not isinstance(manifest, dict):
        raise ValueError("resolved manifest must be a JSON object")

    artifacts = report.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise ValueError("release input report contains no artifacts")
    verified = []
    seen: set[Path] = set()
    for row in artifacts:
        if not isinstance(row, dict):
            raise ValueError("release input artifact row is malformed")
        relative = _safe_relative(row.get("path"))
        if relative in seen:
            raise ValueError(f"duplicate release input path: {relative}")
        seen.add(relative)
        path = input_dir / relative
        payload = path.read_bytes()
        expected_bytes = row.get("bytes")
        expected_sha256 = row.get("sha256")
        expected_blake3 = row.get("blake3")
        if len(payload) != expected_bytes:
            raise ValueError(f"{relative} byte size mismatch")
        if hashlib.sha256(payload).hexdigest() != expected_sha256:
            raise ValueError(f"{relative} SHA-256 mismatch")
        if blake3.blake3(payload).hexdigest() != expected_blake3:
            raise ValueError(f"{relative} BLAKE3 mismatch")
        verified.append(str(relative))
    return {
        "schema": "capsem.release_inputs_verification.v1",
        "ok": True,
        "kind": kind,
        "manifest": str(manifest_path),
        "verified": verified,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        report = verify_release_inputs(args.input_dir)
    except (OSError, ValueError) as error:
        print(f"release input verification failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
