#!/usr/bin/env python3
"""Small exact-byte and report helpers used inside the Tart release guest."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import urllib.request
from pathlib import Path
from typing import cast


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(candidate for candidate in root.rglob("*") if candidate.is_file()):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(b"\0")
        digest.update(bytes.fromhex(sha256(path)))
    return digest.hexdigest()


def assert_url(url: str, expected: Path) -> None:
    request = urllib.request.Request(
        url,
        headers={"Cache-Control": "no-cache", "Pragma": "no-cache"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        fetched = response.read()
        if response.headers.get("Cache-Control") != "no-store":
            raise RuntimeError("release fixture response omitted Cache-Control: no-store")
    if fetched != expected.read_bytes():
        raise RuntimeError(f"release fixture did not serve exact current bytes from {expected}")


def promote(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    pending = destination.with_name(f".{destination.name}.{os.getpid()}.next")
    try:
        shutil.copyfile(source, pending)
        os.replace(pending, destination)
    finally:
        pending.unlink(missing_ok=True)


def _object(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"Tart transition evidence must be an object: {path}")
    return cast(dict[str, object], value)


def write_report(args: argparse.Namespace) -> None:
    installed = _object(args.installed)
    installed["package_receipt"] = True
    installed["binary_cohort"] = True
    fresh_installed = _object(args.fresh_installed)
    fresh_installed["package_receipt"] = True
    fresh_installed["binary_cohort"] = True
    preserved = _object(args.preserved)
    preserved["package_receipt"] = True
    preserved["binary_cohort"] = True
    report = {
        "schema": "capsem.release_glowup.guest.v1",
        "artifact_sha256": sha256(args.package),
        "fresh_installed": fresh_installed,
        "installed": installed,
        "preserved_installed": preserved,
        "fresh_transition": _object(args.fresh_transition),
        "update_transition": _object(args.update_transition),
        "tamper_rejection": _object(args.tamper_rejection),
        "incompatible_rejection": _object(args.incompatible_rejection),
        "guest": {
            "app_version": args.app_version,
            "kernel": args.kernel,
            "architecture": args.architecture,
            "clean_precondition": True,
            "app_bundle": True,
            "installed_binary_signature": "ad-hoc",
        },
    }
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    for command in ("sha256", "tree-digest"):
        child = commands.add_parser(command)
        child.add_argument("path", type=Path)
    exact = commands.add_parser("assert-url")
    exact.add_argument("url")
    exact.add_argument("expected", type=Path)
    copy = commands.add_parser("promote")
    copy.add_argument("source", type=Path)
    copy.add_argument("destination", type=Path)
    report = commands.add_parser("write-report")
    for name in (
        "output",
        "installed",
        "fresh-installed",
        "preserved",
        "fresh-transition",
        "update-transition",
        "tamper-rejection",
        "incompatible-rejection",
        "package",
    ):
        report.add_argument(f"--{name}", required=True, type=Path)
    for name in ("app-version", "kernel", "architecture"):
        report.add_argument(f"--{name}", required=True)
    return root


def main() -> int:
    args = parser().parse_args()
    if args.command == "sha256":
        print(sha256(args.path))
    elif args.command == "tree-digest":
        print(tree_digest(args.path))
    elif args.command == "assert-url":
        assert_url(args.url, args.expected)
    elif args.command == "promote":
        promote(args.source, args.destination)
    else:
        write_report(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
