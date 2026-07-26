#!/usr/bin/env python3
"""Run cargo-audit and make actionable warning classes release-blocking."""

from __future__ import annotations

import json
import subprocess
import sys
from typing import Any


BLOCKING_WARNING_KINDS = frozenset({"unsound", "yanked"})


def validate_report(report: dict[str, Any]) -> dict[str, int]:
    vulnerabilities = report.get("vulnerabilities")
    if not isinstance(vulnerabilities, dict):
        raise ValueError("cargo audit report is missing vulnerabilities")
    count = vulnerabilities.get("count")
    if not isinstance(count, int):
        raise ValueError("cargo audit vulnerability count is invalid")
    if count:
        raise ValueError(f"cargo audit reported {count} vulnerabilities")

    warnings = report.get("warnings")
    if not isinstance(warnings, dict):
        raise ValueError("cargo audit report is missing warnings")
    counts: dict[str, int] = {}
    blocking: list[str] = []
    for kind, entries in sorted(warnings.items()):
        if not isinstance(entries, list):
            raise ValueError(f"cargo audit warning list is invalid: {kind}")
        counts[kind] = len(entries)
        if kind not in BLOCKING_WARNING_KINDS:
            continue
        for entry in entries:
            if not isinstance(entry, dict):
                blocking.append(kind)
                continue
            advisory = entry.get("advisory") or {}
            package = entry.get("package") or {}
            blocking.append(
                " ".join(
                    part
                    for part in (
                        kind,
                        str(advisory.get("id") or "").strip(),
                        str(package.get("name") or "").strip(),
                        str(package.get("version") or "").strip(),
                    )
                    if part
                )
            )
    if blocking:
        raise ValueError(
            "cargo audit reported blocking warnings: " + ", ".join(blocking)
        )
    return counts


def main() -> int:
    result = subprocess.run(
        ["cargo", "audit", "--json"],
        text=True,
        capture_output=True,
        check=False,
    )
    try:
        report = json.loads(result.stdout)
        counts = validate_report(report)
    except (json.JSONDecodeError, ValueError) as error:
        if result.stderr:
            print(result.stderr, file=sys.stderr, end="")
        print(f"strict cargo audit failed: {error}", file=sys.stderr)
        return 1
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr, end="")
        print(
            f"strict cargo audit failed with exit code {result.returncode}",
            file=sys.stderr,
        )
        return 1
    print(
        "strict cargo audit passed: "
        f"nonblocking warnings={sum(counts.values())} {counts}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
