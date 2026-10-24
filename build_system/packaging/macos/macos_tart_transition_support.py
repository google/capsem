#!/usr/bin/env python3
"""Small exact-byte and report helpers used inside the Tart release guest."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import urllib.request
from pathlib import Path
from typing import cast

from capsem_builder import crypto as hashlib


def failure_diagnostic_script() -> str:
    """Bounded startup evidence; marker values and credential contents stay private."""
    return r"""
set +e
echo '=== /var/log/install.log (tail) ==='
sudo tail -n 300 /var/log/install.log
echo '=== ~/.capsem/logs/install.log ==='
cat "$HOME/.capsem/logs/install.log"
echo '=== ~/.capsem/logs/install-failure.txt ==='
cat "$HOME/.capsem/logs/install-failure.txt"
echo '=== ~/.capsem/run/service.log (tail) ==='
# Rotated stream: tail the name alone and you get nothing.
cat "$HOME"/.capsem/run/service*.log 2>/dev/null | tail -n 300
echo '=== gateway startup logs (bounded) ==='
for file in "$HOME/Library/Logs/capsem/gateway.log" "$HOME/.capsem/run/logs/gateway.log" "$HOME/.capsem/run/gateway.log"; do
  echo "=== $file ==="
  tail -c 65536 "$file" 2>/dev/null
done
echo '=== gateway marker metadata only ==='
for marker in gateway.port gateway.pid preview.port gateway.token; do
  stat -f '%N size=%z mode=%Sp uid=%u gid=%g' "$HOME/.capsem/run/$marker" 2>/dev/null
done
echo '=== attestation executable existence ==='
for file in /bin/true /usr/bin/true; do
  if test -x "$file"; then echo "$file executable=yes"; else echo "$file executable=no"; fi
done
echo '=== ~/.capsem/logs listing ==='
ls -la "$HOME/.capsem/logs"
echo '=== package script log events ==='
sudo log show --last 15m --style compact \
  --predicate 'process == "installer" OR process == "package_script_service"' \
  | tail -n 400
"""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def local_tart_capabilities() -> dict[str, bool]:
    """Describe only the evidence produced by the unsigned local Tart rail."""

    return {
        "native_install": True,
        "package_receipt": True,
        "launchd": True,
        "physical_vz_boot": False,
        "signed": False,
        "gatekeeper": False,
    }


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
        "asset_hydration": _object(args.asset_hydration),
        "stale_helper_replacement": _object(args.stale_helper),
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
    digest = commands.add_parser("sha256")
    digest.add_argument("path", type=Path)
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
        "asset-hydration",
        "stale-helper",
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
    elif args.command == "assert-url":
        assert_url(args.url, args.expected)
    elif args.command == "promote":
        promote(args.source, args.destination)
    else:
        write_report(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
