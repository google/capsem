#!/usr/bin/env python3
"""Run cargo-audit and make actionable warning classes release-blocking."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

BLOCKING_WARNING_KINDS = frozenset({"unsound", "yanked"})
_GLIB_FUNCTION_ADVISORY = "RUSTSEC-2024-0429"
_GLIB_AFFECTED_TOKENS = ("array_iter_str", "VariantStrIter")


def _version_triplet(value: object) -> tuple[int, int, int] | None:
    if not isinstance(value, str):
        return None
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value)
    if match is None:
        return None
    return tuple(int(part) for part in match.groups())


def validate_function_scoped_advisories(
    metadata: dict[str, Any],
) -> list[str]:
    """Prove known function-scoped advisories have no resolved caller.

    ``cargo audit`` deliberately omits advisories that name only affected
    functions because a lockfile cannot prove those functions are reachable.
    Dependabot still reports the package range. Keep the exception executable:
    inspect every resolved package's production Rust sources and fail as soon
    as a caller of the affected API enters the graph.
    """
    packages = metadata.get("packages")
    if not isinstance(packages, list):
        raise ValueError("cargo metadata report is missing packages")

    vulnerable_glib_versions: list[str] = []
    for package in packages:
        if not isinstance(package, dict) or package.get("name") != "glib":
            continue
        version = _version_triplet(package.get("version"))
        if version is None:
            raise ValueError("resolved glib package has an invalid version")
        if (0, 15, 0) <= version < (0, 20, 0):
            vulnerable_glib_versions.append(str(package["version"]))
    if not vulnerable_glib_versions:
        return []

    callers: list[str] = []
    for package in packages:
        if not isinstance(package, dict):
            raise ValueError("cargo metadata package row is malformed")
        if package.get("name") == "glib":
            continue
        manifest_value = package.get("manifest_path")
        if not isinstance(manifest_value, str):
            raise ValueError("cargo metadata package row lacks manifest_path")
        source_root = Path(manifest_value).parent / "src"
        if not source_root.is_dir():
            continue
        for source in source_root.rglob("*.rs"):
            try:
                text = source.read_text(encoding="utf-8")
            except OSError as error:
                raise ValueError(f"cannot inspect resolved Rust source {source}: {error}") from error
            if any(token in text for token in _GLIB_AFFECTED_TOKENS):
                callers.append(f"{package.get('name')}:{source}")

    if callers:
        raise ValueError(
            f"{_GLIB_FUNCTION_ADVISORY} affected glib API is referenced by "
            + ", ".join(sorted(callers))
        )
    return [
        f"{_GLIB_FUNCTION_ADVISORY} glib {version}: affected functions unreachable"
        for version in sorted(set(vulnerable_glib_versions))
    ]


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
    metadata_result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        text=True,
        capture_output=True,
        check=False,
    )
    try:
        metadata = json.loads(metadata_result.stdout)
        function_exceptions = validate_function_scoped_advisories(metadata)
    except (json.JSONDecodeError, ValueError) as error:
        if metadata_result.stderr:
            print(metadata_result.stderr, file=sys.stderr, end="")
        print(f"strict cargo audit failed: {error}", file=sys.stderr)
        return 1
    if metadata_result.returncode != 0:
        if metadata_result.stderr:
            print(metadata_result.stderr, file=sys.stderr, end="")
        print(
            "strict cargo audit failed: "
            f"cargo metadata exited with {metadata_result.returncode}",
            file=sys.stderr,
        )
        return 1
    print(
        "strict cargo audit passed: "
        f"nonblocking warnings={sum(counts.values())} {counts}; "
        f"function-scoped proofs={function_exceptions}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
