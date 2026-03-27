"""Vulnerability scanner output parsing.

Parses pre-captured JSON output from trivy and grype. Does NOT execute
scanners -- the CLI layer handles file I/O and process invocation.
"""

from __future__ import annotations

import json

from capsem.builder.manifest import VulnEntry


# ---------------------------------------------------------------------------
# Trivy
# ---------------------------------------------------------------------------


def parse_trivy_json(output: str) -> list[VulnEntry]:
    """Parse trivy JSON output into VulnEntry list."""
    try:
        data = json.loads(output)
    except json.JSONDecodeError as e:
        raise ValueError(f"Invalid trivy JSON: {e}") from e

    entries: list[VulnEntry] = []
    for result in data.get("Results", []):
        for vuln in result.get("Vulnerabilities", []):
            entries.append(VulnEntry(
                id=vuln.get("VulnerabilityID", ""),
                severity=vuln.get("Severity", "UNKNOWN"),
                package=vuln.get("PkgName", ""),
                installed_version=vuln.get("InstalledVersion", ""),
                fixed_version=vuln.get("FixedVersion", ""),
                title=vuln.get("Title", ""),
                scanner="trivy",
            ))
    return entries


# ---------------------------------------------------------------------------
# Grype
# ---------------------------------------------------------------------------


def parse_grype_json(output: str) -> list[VulnEntry]:
    """Parse grype JSON output into VulnEntry list."""
    try:
        data = json.loads(output)
    except json.JSONDecodeError as e:
        raise ValueError(f"Invalid grype JSON: {e}") from e

    entries: list[VulnEntry] = []
    for match in data.get("matches", []):
        vuln = match.get("vulnerability", {})
        artifact = match.get("artifact", {})
        fix = vuln.get("fix", {})

        fix_versions = fix.get("versions", [])
        fix_state = fix.get("state", "")
        fixed = ", ".join(fix_versions) if fix_state == "fixed" and fix_versions else ""

        entries.append(VulnEntry(
            id=vuln.get("id", ""),
            severity=vuln.get("severity", "UNKNOWN").upper(),
            package=artifact.get("name", ""),
            installed_version=artifact.get("version", ""),
            fixed_version=fixed,
            scanner="grype",
        ))
    return entries


# ---------------------------------------------------------------------------
# Dispatcher
# ---------------------------------------------------------------------------


def parse_audit_output(output: str, scanner: str) -> list[VulnEntry]:
    """Parse vulnerability scanner output based on scanner name."""
    if scanner == "trivy":
        return parse_trivy_json(output)
    if scanner == "grype":
        return parse_grype_json(output)
    raise ValueError(f"Unknown scanner: {scanner}")


def summarize_vulns(vulns: list[VulnEntry]) -> dict[str, int]:
    """Count vulnerabilities by severity."""
    counts = {"CRITICAL": 0, "HIGH": 0, "MEDIUM": 0, "LOW": 0, "UNKNOWN": 0}
    for v in vulns:
        key = v.severity.upper()
        if key in counts:
            counts[key] += 1
        else:
            counts["UNKNOWN"] += 1
    return counts
