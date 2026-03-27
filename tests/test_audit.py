"""Tests for capsem.builder.audit -- vulnerability scanner output parsing.

TDD: tests written first (RED), then audit.py makes them pass (GREEN).
"""

from __future__ import annotations

import json

import pytest

from capsem.builder.audit import (
    parse_audit_output,
    parse_grype_json,
    parse_trivy_json,
    summarize_vulns,
)
from capsem.builder.manifest import VulnEntry

# ---------------------------------------------------------------------------
# Inline fixtures
# ---------------------------------------------------------------------------

TRIVY_JSON = json.dumps({
    "Results": [
        {
            "Target": "debian (bookworm)",
            "Vulnerabilities": [
                {
                    "VulnerabilityID": "CVE-2024-1234",
                    "Severity": "HIGH",
                    "PkgName": "openssl",
                    "InstalledVersion": "3.0.13",
                    "FixedVersion": "3.0.14",
                    "Title": "Buffer overflow in SSL_read",
                },
                {
                    "VulnerabilityID": "CVE-2024-5678",
                    "Severity": "MEDIUM",
                    "PkgName": "curl",
                    "InstalledVersion": "7.88.1",
                    "FixedVersion": "",
                    "Title": "HTTP header injection",
                },
            ],
        },
        {
            "Target": "Python (pip)",
            "Vulnerabilities": [
                {
                    "VulnerabilityID": "CVE-2024-9999",
                    "Severity": "CRITICAL",
                    "PkgName": "requests",
                    "InstalledVersion": "2.31.0",
                    "FixedVersion": "2.32.0",
                },
            ],
        },
    ],
})

TRIVY_NO_VULNS = json.dumps({
    "Results": [
        {"Target": "debian (bookworm)"},
    ],
})

TRIVY_EMPTY_RESULTS = json.dumps({"Results": []})

GRYPE_JSON = json.dumps({
    "matches": [
        {
            "vulnerability": {
                "id": "CVE-2024-1111",
                "severity": "High",
                "fix": {"versions": ["1.2.4"], "state": "fixed"},
            },
            "artifact": {"name": "zlib", "version": "1.2.3"},
        },
        {
            "vulnerability": {
                "id": "CVE-2024-2222",
                "severity": "Low",
                "fix": {"versions": ["3.0.1", "3.1.0"], "state": "fixed"},
            },
            "artifact": {"name": "libxml2", "version": "2.9.14"},
        },
        {
            "vulnerability": {
                "id": "CVE-2024-3333",
                "severity": "Medium",
                "fix": {"versions": [], "state": "not-fixed"},
            },
            "artifact": {"name": "bash", "version": "5.2.15"},
        },
    ],
})

GRYPE_EMPTY = json.dumps({"matches": []})


# ---------------------------------------------------------------------------
# parse_trivy_json
# ---------------------------------------------------------------------------


class TestParseTrivyJson:

    def test_happy_path(self):
        vulns = parse_trivy_json(TRIVY_JSON)
        assert len(vulns) == 3

    def test_fields_mapped(self):
        vulns = parse_trivy_json(TRIVY_JSON)
        v = vulns[0]
        assert v.id == "CVE-2024-1234"
        assert v.severity == "HIGH"
        assert v.package == "openssl"
        assert v.installed_version == "3.0.13"
        assert v.fixed_version == "3.0.14"
        assert v.title == "Buffer overflow in SSL_read"
        assert v.scanner == "trivy"

    def test_missing_fixed_version(self):
        vulns = parse_trivy_json(TRIVY_JSON)
        curl = [v for v in vulns if v.package == "curl"][0]
        assert curl.fixed_version == ""

    def test_missing_title(self):
        vulns = parse_trivy_json(TRIVY_JSON)
        requests_v = [v for v in vulns if v.package == "requests"][0]
        assert requests_v.title == ""

    def test_no_vulnerabilities_key(self):
        vulns = parse_trivy_json(TRIVY_NO_VULNS)
        assert vulns == []

    def test_empty_results(self):
        vulns = parse_trivy_json(TRIVY_EMPTY_RESULTS)
        assert vulns == []

    def test_invalid_json(self):
        with pytest.raises(ValueError, match="Invalid"):
            parse_trivy_json("not json")


# ---------------------------------------------------------------------------
# parse_grype_json
# ---------------------------------------------------------------------------


class TestParseGrypeJson:

    def test_happy_path(self):
        vulns = parse_grype_json(GRYPE_JSON)
        assert len(vulns) == 3

    def test_severity_normalized(self):
        vulns = parse_grype_json(GRYPE_JSON)
        zlib = [v for v in vulns if v.package == "zlib"][0]
        assert zlib.severity == "HIGH"  # "High" -> "HIGH"

    def test_fix_versions_joined(self):
        vulns = parse_grype_json(GRYPE_JSON)
        libxml = [v for v in vulns if v.package == "libxml2"][0]
        assert libxml.fixed_version == "3.0.1, 3.1.0"

    def test_not_fixed(self):
        vulns = parse_grype_json(GRYPE_JSON)
        bash = [v for v in vulns if v.package == "bash"][0]
        assert bash.fixed_version == ""

    def test_scanner_is_grype(self):
        vulns = parse_grype_json(GRYPE_JSON)
        assert all(v.scanner == "grype" for v in vulns)

    def test_empty_matches(self):
        vulns = parse_grype_json(GRYPE_EMPTY)
        assert vulns == []

    def test_invalid_json(self):
        with pytest.raises(ValueError, match="Invalid"):
            parse_grype_json("not json")


# ---------------------------------------------------------------------------
# parse_audit_output
# ---------------------------------------------------------------------------


class TestParseAuditOutput:

    def test_trivy_dispatch(self):
        vulns = parse_audit_output(TRIVY_JSON, "trivy")
        assert len(vulns) == 3
        assert all(v.scanner == "trivy" for v in vulns)

    def test_grype_dispatch(self):
        vulns = parse_audit_output(GRYPE_JSON, "grype")
        assert len(vulns) == 3
        assert all(v.scanner == "grype" for v in vulns)

    def test_unknown_scanner(self):
        with pytest.raises(ValueError, match="Unknown scanner"):
            parse_audit_output("{}", "snyk")


# ---------------------------------------------------------------------------
# summarize_vulns
# ---------------------------------------------------------------------------


class TestSummarizeVulns:

    def test_counts(self):
        vulns = parse_trivy_json(TRIVY_JSON)
        summary = summarize_vulns(vulns)
        assert summary["CRITICAL"] == 1
        assert summary["HIGH"] == 1
        assert summary["MEDIUM"] == 1
        assert summary["LOW"] == 0

    def test_empty(self):
        summary = summarize_vulns([])
        assert summary == {"CRITICAL": 0, "HIGH": 0, "MEDIUM": 0, "LOW": 0, "UNKNOWN": 0}

    def test_mixed(self):
        vulns = parse_grype_json(GRYPE_JSON)
        summary = summarize_vulns(vulns)
        assert summary["HIGH"] == 1
        assert summary["MEDIUM"] == 1
        assert summary["LOW"] == 1
