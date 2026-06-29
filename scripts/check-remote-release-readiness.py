#!/usr/bin/env python3
"""Read-only checks for live release rails before cutting or deploying."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import socket
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlparse

try:
    import blake3
except ModuleNotFoundError as error:
    blake3 = None
    BLAKE3_IMPORT_ERROR = error
else:
    BLAKE3_IMPORT_ERROR = None


REQUIRED_PR_GATE_JOBS = ("test-linux", "test", "test-install", "docs-build", "site-build")
REQUIRED_PR_GATE_RESULT_CHECKS = (
    ("test-linux", "TEST_LINUX_RESULT"),
    ("test", "TEST_MACOS_RESULT"),
    ("test-install", "TEST_INSTALL_RESULT"),
    ("docs-build", "DOCS_BUILD_RESULT"),
    ("site-build", "SITE_BUILD_RESULT"),
)
RELEASE_VALIDATOR_USER_AGENT = "CapsemReleaseValidator/1.0"


@dataclass
class CheckResult:
    name: str
    ok: bool
    detail: str


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Read-only remote release readiness checks for pr-gate, "
            "branch protection, release.capsem.org, and the asset channel."
        )
    )
    parser.add_argument("--repo", default="google/capsem", help="GitHub owner/repo")
    parser.add_argument("--branch", default="main", help="Protected branch to inspect")
    parser.add_argument("--remote", default="origin", help="Git remote to compare with HEAD")
    parser.add_argument("--channel", default="stable", help="Asset channel name")
    parser.add_argument(
        "--release-site",
        default="https://release.capsem.org",
        help="Public asset-channel site root",
    )
    args = parser.parse_args()

    if BLAKE3_IMPORT_ERROR is not None:
        print(
            "missing Python dependency: blake3. Run `uv sync` once, then "
            "`uv run python scripts/check-remote-release-readiness.py`.",
            file=sys.stderr,
        )
        return 2

    checks = [
        check_local_branch_publication(args.remote, args.branch),
        check_remote_pr_gate(args.repo),
        check_remote_branch_protection(args.repo, args.branch),
        check_release_site_dns(args.release_site),
        check_release_site_contract(args.release_site, args.channel),
    ]
    failures = [check for check in checks if not check.ok]

    for check in checks:
        status = "OK" if check.ok else "FAIL"
        print(f"{status}: {check.name}: {check.detail}")

    if failures:
        print(
            "\nRemote release readiness failed. This script is read-only; fix GitHub "
            "branch protection/rulesets, Cloudflare DNS, or the published asset "
            "channel, then rerun.",
            file=sys.stderr,
        )
        return 1
    return 0


def check_local_branch_publication(remote: str, branch: str) -> CheckResult:
    base = f"{remote}/{branch}"
    comparison = run_text(["git", "rev-list", "--left-right", "--count", f"{base}...HEAD"])
    if comparison.returncode != 0:
        return CheckResult(
            "local branch publication",
            False,
            f"cannot compare HEAD with {base}: {comparison.stderr.strip()}",
        )
    try:
        behind, ahead = (int(part) for part in comparison.stdout.split())
    except ValueError:
        return CheckResult(
            "local branch publication",
            False,
            f"unexpected rev-list output for {base}: {comparison.stdout.strip()}",
        )
    if ahead or behind:
        details = []
        if ahead:
            details.append(f"HEAD is ahead of {base} by {ahead} commit(s)")
        if behind:
            details.append(f"HEAD is behind {base} by {behind} commit(s)")
        details.append("publish or merge release-rail commits before claiming remote readiness")
        return CheckResult("local branch publication", False, "; ".join(details))
    return CheckResult("local branch publication", True, f"HEAD matches {base}")


def check_remote_pr_gate(repo: str) -> CheckResult:
    workflow = run_text(["gh", "workflow", "view", "ci.yaml", "--repo", repo, "--yaml"])
    if workflow.returncode != 0:
        return CheckResult("remote ci.yaml pr-gate", False, workflow.stderr.strip())
    text = workflow.stdout
    if "pr-gate:" not in text:
        return CheckResult("remote ci.yaml pr-gate", False, "remote ci.yaml lacks pr-gate")
    pr_gate = workflow_job_block(text, "pr-gate")
    failures = pr_gate_contract_failures(pr_gate)
    if failures:
        return CheckResult(
            "remote ci.yaml pr-gate",
            False,
            "remote pr-gate is not fail-closed: " + "; ".join(failures),
        )
    return CheckResult(
        "remote ci.yaml pr-gate",
        True,
        "pr-gate aggregates required jobs and asserts all results",
    )


def workflow_job_block(workflow: str, name: str) -> str:
    lines = workflow.splitlines()
    start = next((i for i, line in enumerate(lines) if line == f"  {name}:"), None)
    if start is None:
        return ""
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = i
            break
    return "\n".join(lines[start:end])


def workflow_job_needs(job_block: str) -> set[str]:
    inline = re.search(r"(?m)^\s+needs:\s*\[([^\]]+)\]\s*$", job_block)
    if inline:
        return {part.strip() for part in inline.group(1).split(",") if part.strip()}

    needs: set[str] = set()
    lines = job_block.splitlines()
    for i, line in enumerate(lines):
        if re.match(r"^\s+needs:\s*$", line):
            for item in lines[i + 1 :]:
                if not item.startswith("      - "):
                    break
                needs.add(item.removeprefix("      - ").strip())
            break
    return needs


def pr_gate_contract_failures(job_block: str) -> list[str]:
    failures: list[str] = []
    missing = sorted(set(REQUIRED_PR_GATE_JOBS) - workflow_job_needs(job_block))
    if missing:
        failures.append("does not aggregate required jobs: " + ", ".join(missing))
    if not re.search(r"(?m)^\s+if:\s*\$\{\{\s*always\(\)\s*\}}\s*$", job_block):
        failures.append("pr-gate does not run with if: ${{ always() }}")
    for job, env_name in REQUIRED_PR_GATE_RESULT_CHECKS:
        if f"needs.{job}.result" not in job_block or not result_success_asserted(
            job_block, env_name
        ):
            failures.append(f"pr-gate does not assert {job} result")
    return failures


def result_success_asserted(job_block: str, env_name: str) -> bool:
    return re.search(
        rf"(?m)^\s*(?:test|\[)\s+[\"']?\${re.escape(env_name)}[\"']?\s*=\s*success",
        job_block,
    ) is not None


def check_remote_branch_protection(repo: str, branch: str) -> CheckResult:
    classic = run_json(["gh", "api", f"repos/{repo}/branches/{branch}/protection"])
    classic_required = False
    classic_detail = ""
    if classic.returncode == 0 and classic.data is not None:
        classic_required = classic_protection_requires_pr_gate(classic.data)
        classic_detail = "classic branch protection"

    active_rules = run_json(["gh", "api", f"repos/{repo}/rules/branches/{branch}"])
    active_rules_required = False
    if active_rules.returncode == 0 and active_rules.data is not None:
        active_rules_required = active_branch_rules_require_pr_gate(active_rules.data)

    if classic_required or active_rules_required:
        sources = []
        if classic_required:
            sources.append(classic_detail)
        if active_rules_required:
            sources.append("active branch rules")
        return CheckResult(
            "remote branch protection requires pr-gate",
            True,
            ", ".join(sources),
        )

    detail = "pr-gate is not required by classic branch protection or active branch rules"
    if classic.returncode != 0:
        detail += f"; classic protection probe: {classic.stderr.strip()}"
    if active_rules.returncode != 0:
        detail += f"; active branch rules probe: {active_rules.stderr.strip()}"
    return CheckResult("remote branch protection requires pr-gate", False, detail)


def check_release_site_dns(release_site: str) -> CheckResult:
    host = urlparse(release_site).hostname
    if not host:
        return CheckResult("release.capsem.org DNS", False, f"invalid URL: {release_site}")
    try:
        socket.getaddrinfo(host, 443)
    except socket.gaierror as error:
        return CheckResult("release.capsem.org DNS", False, str(error))
    return CheckResult("release.capsem.org DNS", True, f"{host} resolves")


def check_release_site_contract(release_site: str, channel: str) -> CheckResult:
    site = release_site.rstrip("/")
    index_url = f"{site}/"
    health_url = f"{site}/health.json"
    manifest_path = f"/assets/{channel}/manifest.json"
    manifest_url = f"{site}{manifest_path}"

    index = fetch_text(index_url)
    if index.error:
        return CheckResult("release.capsem.org contract", False, index.error)
    health = fetch_json(health_url)
    if health.error:
        return CheckResult("release.capsem.org contract", False, health.error)
    manifest = fetch_json(manifest_url)
    if manifest.error:
        return CheckResult("release.capsem.org contract", False, manifest.error)

    failures: list[str] = []
    health_data = health.data if isinstance(health.data, dict) else {}
    manifest_data = manifest.data if isinstance(manifest.data, dict) else {}
    health_urls = require_object(health_data, "urls", "health urls", failures)
    health_current = require_object(health_data, "current", "health current", failures)
    health_binary = require_object(health_data, "binary", "health binary", failures)
    health_assets = require_object(health_data, "assets", "health assets", failures)
    health_profiles = require_object(health_data, "profiles", "health profiles", failures)
    manifest_assets = require_object(manifest_data, "assets", "manifest assets", failures)
    manifest_binaries = require_object(manifest_data, "binaries", "manifest binaries", failures)
    manifest_asset_releases = require_object(
        manifest_assets, "releases", "manifest asset releases", failures
    )
    manifest_binary_releases = require_object(
        manifest_binaries, "releases", "manifest binary releases", failures
    )

    if health_data.get("schema") != "capsem.assets_channel.health.v1":
        failures.append("health schema mismatch")
    if health_data.get("ok") is not True:
        failures.append("health ok mismatch")
    if health_data.get("channel") != channel:
        failures.append("health channel mismatch")
    if health_data.get("state") != "published":
        failures.append("health state mismatch")
    if health_urls.get("index") != "/index.html":
        failures.append("health index URL mismatch")
    if health_urls.get("health") != "/health.json":
        failures.append("health health URL mismatch")
    if health_urls.get("manifest") != manifest_path:
        failures.append("health manifest URL mismatch")
    if health_urls.get("asset_base") != "/assets/releases":
        failures.append("health asset base mismatch")
    if manifest_data.get("format") != 2:
        failures.append("manifest format mismatch")

    current_binary = health_current.get("binary")
    current_assets = health_current.get("assets")
    profile_source = health_profiles.get("source")
    current_manifest_asset_release = (
        manifest_asset_releases.get(current_assets)
        if isinstance(manifest_asset_releases, dict)
        else None
    )
    current_manifest_binary_release = (
        manifest_binary_releases.get(current_binary)
        if isinstance(manifest_binary_releases, dict)
        else None
    )
    health_updates = health_data.get("updates")
    health_update_binary = (
        health_updates.get("binary") if isinstance(health_updates, dict) else None
    )
    health_update_assets = (
        health_updates.get("assets") if isinstance(health_updates, dict) else None
    )
    health_update_profiles = (
        health_updates.get("profiles") if isinstance(health_updates, dict) else None
    )
    health_update_images = (
        health_updates.get("images") if isinstance(health_updates, dict) else None
    )
    profile_update_source = (
        health_update_profiles.get("source") if isinstance(health_update_profiles, dict) else None
    )
    if health_urls.get("profile_catalog") != profile_source:
        failures.append("health profile catalog URL mismatch")
    if profile_update_source != profile_source:
        failures.append("health profile update source mismatch")
    if health_binary.get("version") != current_binary:
        failures.append("health binary version mismatch")
    if health_assets.get("version") != current_assets:
        failures.append("health asset version mismatch")
    expected_asset_compatibility = {
        "binary": current_binary,
        "min_binary": current_manifest_asset_release.get("min_binary")
        if isinstance(current_manifest_asset_release, dict)
        else None,
    }
    actual_asset_compatibility = health_assets.get("compatibility")
    for field, expected in expected_asset_compatibility.items():
        actual = (
            actual_asset_compatibility.get(field)
            if isinstance(actual_asset_compatibility, dict)
            else None
        )
        if actual != expected:
            failures.append(f"health asset compatibility {field} mismatch")
    actual_asset_requires_newer = health_assets.get("requires_newer")
    actual_asset_requires_newer_binary = (
        actual_asset_requires_newer.get("binary")
        if isinstance(actual_asset_requires_newer, dict)
        else None
    )
    if actual_asset_requires_newer_binary is not False:
        failures.append("health asset requirement binary mismatch")
    if health_profiles.get("state") != "current":
        failures.append("health profile state mismatch")
    expected_profile_compatibility = {
        "binary": current_binary,
        "assets": current_assets,
        "min_binary": current_manifest_asset_release.get("min_binary")
        if isinstance(current_manifest_asset_release, dict)
        else None,
        "min_assets": current_manifest_binary_release.get("min_assets")
        if isinstance(current_manifest_binary_release, dict)
        else None,
    }
    actual_profile_compatibility = health_profiles.get("compatibility")
    for field, expected in expected_profile_compatibility.items():
        actual = (
            actual_profile_compatibility.get(field)
            if isinstance(actual_profile_compatibility, dict)
            else None
        )
        if actual != expected:
            failures.append(f"health profile compatibility {field} mismatch")
    actual_profile_requires_newer = health_profiles.get("requires_newer")
    for field, expected in (("binary", False), ("assets", False)):
        actual = (
            actual_profile_requires_newer.get(field)
            if isinstance(actual_profile_requires_newer, dict)
            else None
        )
        if actual != expected:
            failures.append(f"health profile requirement {field} mismatch")
    if not isinstance(health_update_binary, dict):
        failures.append("health binary update metadata missing")
    else:
        if health_update_binary.get("latest") != current_binary:
            failures.append("health binary update latest mismatch")
        if health_update_binary.get("current") != current_binary:
            failures.append("health binary update current mismatch")
        if health_update_binary.get("state") != health_binary.get("state"):
            failures.append("health binary update state mismatch")
        if health_update_binary.get("source") != "manifest.binaries.current":
            failures.append("health binary update source mismatch")
        if health_update_binary.get("files") != health_binary.get("files"):
            failures.append("health binary update files mismatch")
    if not isinstance(health_update_profiles, dict):
        failures.append("health profile update metadata missing")
    else:
        if health_update_profiles.get("hash") != health_profiles.get("hash"):
            failures.append("health profile update hash mismatch")
        if health_update_profiles.get("compatibility") != health_profiles.get("compatibility"):
            failures.append("health profile update compatibility mismatch")
        if health_update_profiles.get("requires_newer") != health_profiles.get("requires_newer"):
            failures.append("health profile update requirement mismatch")
    failures.extend(
        check_profile_catalog_artifact(
            site,
            profile_source,
            health_profiles.get("hash"),
            health_profiles.get("revision"),
            current_binary,
            current_assets,
            expected_profile_compatibility,
            {"binary": False, "assets": False},
        )
    )
    if not isinstance(health_update_assets, dict):
        failures.append("health asset update metadata missing")
    else:
        if health_update_assets.get("latest") != current_assets:
            failures.append("health asset update latest mismatch")
        if health_update_assets.get("current") != current_assets:
            failures.append("health asset update current mismatch")
        if health_update_assets.get("state") != health_assets.get("state"):
            failures.append("health asset update state mismatch")
        if health_update_assets.get("source") != "manifest.assets.current":
            failures.append("health asset update source mismatch")
        if health_update_assets.get("manifest") != manifest_path:
            failures.append("health asset update manifest mismatch")
        if health_update_assets.get("asset_base") != "/assets/releases":
            failures.append("health asset update base mismatch")
        if health_update_assets.get("compatibility") != health_assets.get("compatibility"):
            failures.append("health asset update compatibility mismatch")
        if health_update_assets.get("requires_newer") != health_assets.get("requires_newer"):
            failures.append("health asset update requirement mismatch")
        if health_update_assets.get("compatibility") != expected_asset_compatibility:
            failures.append("health asset update canonical compatibility mismatch")
        if health_update_assets.get("requires_newer") != {"binary": False}:
            failures.append("health asset update canonical requirement mismatch")
    if not isinstance(health_update_images, dict):
        failures.append("health image update metadata missing")
    else:
        if health_update_images.get("latest") is not None:
            failures.append("health image update latest must be null while unpublished")
        if health_update_images.get("current") is not None:
            failures.append("health image update current must be null while unpublished")
        if health_update_images.get("state") != "not_published":
            failures.append("health image update state mismatch")
        if health_update_images.get("source") != "not_in_asset_channel":
            failures.append("health image update source mismatch")
    if manifest_assets.get("current") != current_assets:
        failures.append("current asset mismatch between health and manifest")
    if manifest_binaries.get("current") != current_binary:
        failures.append("current binary mismatch between health and manifest")
    expected_asset_files = current_asset_file_refs(
        current_assets,
        current_manifest_asset_release,
        failures,
    )
    failures.extend(
        check_health_asset_files(
            health_assets.get("files"),
            expected_asset_files,
        )
    )
    expected_binary_files = current_binary_file_refs(
        current_binary,
        current_manifest_binary_release,
        failures,
    )
    failures.extend(
        check_host_binary_files(
            health_binary.get("files"),
            expected_binary_files,
            "health",
        )
    )
    evidence_data = health_data.get("evidence")
    evidence_host_binary_files = (
        evidence_data.get("host_binary_files") if isinstance(evidence_data, dict) else None
    )
    failures.extend(
        check_host_binary_files(
            evidence_host_binary_files,
            expected_binary_files,
            "evidence",
        )
    )
    for label, value in (
        ("current binary", current_binary),
        ("current assets", current_assets),
        ("generated timestamp", health_data.get("generated_at")),
        ("profile revision", health_profiles.get("revision")),
        ("profile catalog", profile_source),
        ("channel manifest", manifest_path),
    ):
        if not isinstance(value, str):
            failures.append(f"health {label} missing")
        elif value not in index.text:
            failures.append(f"index missing {label} {value}")

    health_asset_releases = health_data.get("asset_releases")
    if not isinstance(health_asset_releases, list):
        failures.append("health asset releases missing or not a list")
        health_asset_releases = []
    asset_release_by_version = {
        release.get("version"): release
        for release in health_asset_releases
        if isinstance(release, dict) and isinstance(release.get("version"), str)
    }
    for version, manifest_release in manifest_asset_releases.items():
        if not isinstance(version, str) or not isinstance(manifest_release, dict):
            failures.append("manifest asset release entry malformed")
            continue
        public_release = asset_release_by_version.get(version)
        if not isinstance(public_release, dict):
            failures.append(f"health missing asset release {version}")
            continue
        expected_deprecated = manifest_release.get("deprecated", False)
        expected_state = "deprecated" if expected_deprecated is True else "current"
        expected_fields = (
            ("date", manifest_release.get("date")),
            ("state", expected_state),
            ("deprecated", expected_deprecated),
            ("deprecated_date", manifest_release.get("deprecated_date")),
            ("min_binary", manifest_release.get("min_binary")),
        )
        for field, expected in expected_fields:
            if public_release.get(field) != expected:
                failures.append(f"health asset release {version} {field} mismatch")

    current_asset_release = asset_release_by_version.get(current_assets, {})
    current_asset_date = current_asset_release.get("date")
    if not isinstance(current_asset_date, str):
        failures.append("current asset release date missing")
    elif current_asset_date not in index.text:
        failures.append("index missing current asset release date")

    failures.extend(check_release_evidence(site, health_data))
    failures.extend(check_release_cache_headers(site, channel, health_data))

    if failures:
        return CheckResult("release.capsem.org contract", False, "; ".join(failures))
    return CheckResult(
        "release.capsem.org contract",
        True,
        "index, health.json, manifest, evidence artifacts, and cache headers agree",
    )


def check_release_evidence(site: str, health: dict[str, Any]) -> list[str]:
    evidence = health.get("evidence")
    if not isinstance(evidence, dict):
        return ["health evidence missing"]

    failures: list[str] = []
    vm_oboms = require_list(evidence, "vm_oboms", failures)
    host_sboms = require_list(evidence, "host_sboms", failures)
    host_binary_files = require_list(evidence, "host_binary_files", failures)
    attestations = require_list(evidence, "attestations", failures)
    asset_files = require_list(health.get("assets", {}), "files", failures)

    host_binary_by_url = entries_by_url(host_binary_files, failures, "host binary file")
    asset_by_url = entries_by_url(asset_files, failures, "asset file")
    host_package_subjects = {
        url
        for url, item in host_binary_by_url.items()
        if item.get("name") != "capsem-sbom.spdx.json"
    }
    host_sbom_urls = {
        item["url"]
        for item in host_sboms
        if isinstance(item, dict) and isinstance(item.get("url"), str)
    }
    vm_obom_urls = {
        item["url"]
        for item in vm_oboms
        if isinstance(item, dict) and isinstance(item.get("url"), str)
    }

    if host_binary_files and not host_sboms:
        failures.append("health evidence host_sboms missing for published binary files")
    if asset_files and not vm_oboms:
        failures.append("health evidence vm_oboms missing for published VM assets")
    if (host_binary_files or asset_files) and not attestations:
        failures.append("health evidence attestations missing for published artifacts")

    for sbom in host_sboms:
        if not isinstance(sbom, dict):
            failures.append("health evidence host_sboms entry is not an object")
            continue
        url = sbom.get("url")
        if not isinstance(url, str):
            failures.append("health evidence host_sboms entry missing url")
            continue
        if sbom.get("name") != "capsem-sbom.spdx.json":
            failures.append(f"host SBOM evidence {url} name mismatch")
        host_binary = host_binary_by_url.get(url)
        if host_binary is None:
            failures.append(f"host SBOM evidence {url} missing from host binary files")
            continue
        if host_binary.get("name") != "capsem-sbom.spdx.json":
            failures.append(f"host SBOM evidence {url} binary file name mismatch")
        failures.extend(
            fetch_and_verify_evidence_artifact(
                site, sbom, "sha256", "host SBOM evidence", "spdx"
            )
        )

    for obom in vm_oboms:
        if not isinstance(obom, dict):
            failures.append("health evidence vm_oboms entry is not an object")
            continue
        url = obom.get("url")
        if not isinstance(url, str):
            failures.append("health evidence vm_oboms entry missing url")
            continue
        if url not in asset_by_url:
            failures.append(f"VM OBOM evidence {url} missing from asset files")
            continue
        failures.extend(
            fetch_and_verify_evidence_artifact(
                site, obom, "blake3", "VM OBOM evidence", "cyclonedx"
            )
        )

    saw_host_sbom_attestation = False
    host_sbom_attestation_subjects: set[str] = set()
    for attestation in attestations:
        if not isinstance(attestation, dict):
            failures.append("health evidence attestations entry is not an object")
            continue
        attestation_name = attestation.get("name")
        expected_rail = attestation_expected_rails().get(attestation_name)
        if expected_rail is not None:
            scope = attestation.get("scope")
            if scope != expected_rail["scope"]:
                failures.append(f"health evidence {attestation_name} scope mismatch")
            workflow = attestation.get("workflow")
            if workflow != expected_rail["workflow"]:
                failures.append(f"health evidence {attestation_name} workflow mismatch")
        if attestation_name == "github_attestations_host_sbom":
            saw_host_sbom_attestation = True
        predicate_type = attestation.get("predicate_type")
        if not isinstance(predicate_type, str) or not predicate_type:
            failures.append("health evidence attestation predicate_type missing")
        verify_command = attestation.get("verify_command")
        if not isinstance(verify_command, str) or "gh attestation verify" not in verify_command:
            failures.append("health evidence attestation verify_command must use gh attestation verify")
        predicate_url = attestation.get("predicate_url")
        subjects = attestation.get("subjects")
        if not isinstance(subjects, list) or not subjects:
            failures.append("health evidence attestation subjects missing")
            continue
        if attestation_name == "github_attestations_host_sbom" and not isinstance(
            predicate_url, str
        ):
            failures.append("health evidence host SBOM attestation predicate_url missing")
        if attestation_name == "github_attestations_vm_assets" and not isinstance(
            predicate_url, str
        ):
            failures.append("health evidence VM asset attestation predicate_url missing")
        predicate_urls, predicate_label = attestation_predicate_evidence_urls(
            attestation,
            subjects,
            host_binary_by_url,
            asset_by_url,
            host_sbom_urls,
            vm_obom_urls,
        )
        if predicate_url is not None and predicate_url not in predicate_urls:
            failures.append(f"attestation predicate_url {predicate_url} missing from {predicate_label}")
        for subject in subjects:
            if not isinstance(subject, str):
                failures.append("health evidence attestation subject is not a string")
                continue
            if attestation_name == "github_attestations_host_sbom":
                host_sbom_attestation_subjects.add(subject)
            if subject not in host_binary_by_url and subject not in asset_by_url:
                failures.append(f"attestation subject {subject} missing from published file lists")
    if host_sboms and not saw_host_sbom_attestation:
        failures.append("health evidence host SBOM attestation missing")
    for subject in sorted(host_package_subjects - host_sbom_attestation_subjects):
        failures.append(f"health evidence host SBOM attestation subjects missing {subject}")

    return failures


def attestation_expected_rails() -> dict[str, dict[str, str]]:
    return {
        "github_attestations_host": {
            "scope": "host_binaries",
            "workflow": ".github/workflows/release.yaml",
        },
        "github_attestations_host_sbom": {
            "scope": "host_sbom",
            "workflow": ".github/workflows/release.yaml",
        },
        "github_attestations_vm_assets": {
            "scope": "vm_assets",
            "workflow": ".github/workflows/release-assets.yaml",
        },
    }


def attestation_predicate_evidence_urls(
    attestation: dict[str, Any],
    subjects: list[Any],
    host_binary_by_url: dict[str, dict[str, Any]],
    asset_by_url: dict[str, dict[str, Any]],
    host_sbom_urls: set[str],
    vm_obom_urls: set[str],
) -> tuple[set[str], str]:
    scope = attestation.get("scope")
    if scope == "vm_assets":
        return vm_obom_urls, "VM OBOM evidence"
    if scope == "host_binaries":
        return host_sbom_urls, "host SBOM evidence"

    string_subjects = {subject for subject in subjects if isinstance(subject, str)}
    has_vm_asset_subject = any(subject in asset_by_url for subject in string_subjects)
    has_host_binary_subject = any(subject in host_binary_by_url for subject in string_subjects)
    if has_vm_asset_subject and not has_host_binary_subject:
        return vm_obom_urls, "VM OBOM evidence"
    if has_host_binary_subject and not has_vm_asset_subject:
        return host_sbom_urls, "host SBOM evidence"
    return host_sbom_urls | vm_obom_urls, "host SBOM or VM OBOM evidence"


def require_object(
    root: Any, key: str, label: str, failures: list[str]
) -> dict[str, Any]:
    if not isinstance(root, dict):
        failures.append(f"{label} parent is not an object")
        return {}
    value = root.get(key)
    if not isinstance(value, dict):
        failures.append(f"{label} missing or not an object")
        return {}
    return value


def require_list(root: Any, key: str, failures: list[str]) -> list[Any]:
    if not isinstance(root, dict):
        failures.append(f"health evidence {key} parent is not an object")
        return []
    value = root.get(key)
    if not isinstance(value, list):
        failures.append(f"health evidence {key} missing or not a list")
        return []
    return value


def current_asset_file_refs(
    asset_version: Any,
    release: Any,
    failures: list[str],
) -> list[dict[str, Any]]:
    if not isinstance(asset_version, str):
        return []
    if not isinstance(release, dict):
        failures.append("manifest current asset release missing or not an object")
        return []
    arches = release.get("arches")
    if not isinstance(arches, dict):
        failures.append("manifest current asset release arches missing or not an object")
        return []

    refs: list[dict[str, Any]] = []
    for arch, assets in arches.items():
        if not isinstance(arch, str) or not isinstance(assets, dict):
            failures.append("manifest current asset arch entry malformed")
            continue
        for logical_name, entry in assets.items():
            if not isinstance(logical_name, str) or not isinstance(entry, dict):
                failures.append("manifest current asset file entry malformed")
                continue
            url = f"/assets/releases/{asset_version}/{arch}-{logical_name}"
            hash_value = entry.get("hash")
            size = entry.get("size")
            if not isinstance(hash_value, str):
                failures.append(f"manifest asset file {url} hash missing")
                continue
            if not isinstance(size, int):
                failures.append(f"manifest asset file {url} size missing")
                continue
            refs.append(
                {
                    "arch": arch,
                    "logical_name": logical_name,
                    "url": url,
                    "hash": hash_value,
                    "size": size,
                }
            )
    return refs


def check_health_asset_files(
    asset_files: Any,
    expected_asset_files: list[dict[str, Any]],
) -> list[str]:
    if not isinstance(asset_files, list):
        return ["health asset files missing or not a list"]

    failures: list[str] = []
    files_by_url: dict[str, dict[str, Any]] = {}
    for item in asset_files:
        if not isinstance(item, dict):
            failures.append("health asset file entry is not an object")
            continue
        url = item.get("url")
        if not isinstance(url, str):
            failures.append("health asset file entry missing url")
            continue
        files_by_url[url] = item

    expected_urls = {item["url"] for item in expected_asset_files}
    for expected in expected_asset_files:
        url = expected["url"]
        public_file = files_by_url.get(url)
        if public_file is None:
            failures.append(f"health missing asset file {url}")
            continue
        for field in ("arch", "logical_name", "hash", "size"):
            if public_file.get(field) != expected[field]:
                failures.append(f"health asset {field} mismatch for {url}")

    for url in sorted(set(files_by_url) - expected_urls):
        failures.append(f"health unexpected asset file {url}")

    return failures


def current_binary_file_refs(
    binary_version: Any,
    release: Any,
    failures: list[str],
) -> list[dict[str, Any]]:
    if not isinstance(binary_version, str):
        return []
    if not isinstance(release, dict):
        failures.append("manifest current binary release missing or not an object")
        return []
    files = release.get("files", [])
    if not isinstance(files, list):
        failures.append("manifest current binary release files missing or not a list")
        return []

    base = os.environ.get("CAPSEM_RELEASE_URL") or (
        "https://github.com/google/capsem/releases/download"
    )
    release_base = f"{base.rstrip('/')}/v{binary_version}"
    refs: list[dict[str, Any]] = []
    for item in files:
        if not isinstance(item, dict):
            failures.append("manifest binary file entry is not an object")
            continue
        name = item.get("name")
        sha256 = item.get("sha256")
        size = item.get("size")
        if not isinstance(name, str):
            failures.append("manifest binary file name missing")
            continue
        url = f"{release_base}/{name}"
        if not isinstance(sha256, str):
            failures.append(f"manifest binary file {url} sha256 missing")
            continue
        if not isinstance(size, int):
            failures.append(f"manifest binary file {url} size missing")
            continue
        refs.append({"name": name, "url": url, "sha256": sha256, "size": size})
    return refs


def check_host_binary_files(
    binary_files: Any,
    expected_binary_files: list[dict[str, Any]],
    label: str,
) -> list[str]:
    if not isinstance(binary_files, list):
        return [f"{label} host binary files missing or not a list"]

    failures: list[str] = []
    files_by_url: dict[str, dict[str, Any]] = {}
    for item in binary_files:
        if not isinstance(item, dict):
            failures.append(f"{label} host binary file entry is not an object")
            continue
        url = item.get("url")
        if not isinstance(url, str):
            failures.append(f"{label} host binary file entry missing url")
            continue
        files_by_url[url] = item

    expected_urls = {item["url"] for item in expected_binary_files}
    for expected in expected_binary_files:
        url = expected["url"]
        public_file = files_by_url.get(url)
        if public_file is None:
            failures.append(f"{label} missing host binary file {url}")
            continue
        for field in ("name", "sha256", "size"):
            if public_file.get(field) != expected[field]:
                failures.append(f"{label} host binary {field} mismatch for {url}")

    for url in sorted(set(files_by_url) - expected_urls):
        failures.append(f"{label} unexpected host binary file {url}")

    return failures


def check_profile_catalog_artifact(
    site: str,
    source: Any,
    expected_hash: Any,
    expected_revision: Any,
    expected_current_binary: Any,
    expected_current_assets: Any,
    expected_compatibility: dict[str, Any],
    expected_requires_newer: dict[str, bool],
) -> list[str]:
    if not isinstance(source, str):
        return ["profile catalog source missing or not a string"]
    if not source.startswith("/profiles/releases/") or not source.endswith("/catalog.json"):
        return ["profile catalog source must be a release-channel artifact path"]
    if not isinstance(expected_hash, str):
        return [f"profile catalog {source} hash missing or not a string"]
    if not isinstance(expected_revision, str):
        return [f"profile catalog {source} revision missing or not a string"]

    try:
        resolved_url = resolve_release_url(site, source)
    except ValueError as error:
        return [f"profile catalog {source}: {error}"]

    catalog = fetch_bytes(resolved_url)
    if catalog.error:
        return [catalog.error]

    failures: list[str] = []
    if blake3 is None:
        failures.append(
            f"profile catalog {source} cannot verify blake3 without Python dependency blake3"
        )
    else:
        actual_hash = blake3.blake3(catalog.data).hexdigest()
        if actual_hash != expected_hash:
            failures.append(f"profile catalog {source} blake3 mismatch")

    try:
        text = catalog.data.decode("utf-8")
    except UnicodeDecodeError:
        return failures + [f"profile catalog {source} is not UTF-8"]
    if "file://" in text:
        failures.append(f"profile catalog {source} must not contain file:// URLs")

    try:
        document = json.loads(text)
    except json.JSONDecodeError as error:
        return failures + [f"profile catalog {source} JSON parse failed: {error}"]
    if not isinstance(document, dict):
        failures.append(f"profile catalog {source} document is not an object")
        return failures
    if document.get("schema") != "capsem.profile_catalog.v1":
        failures.append(f"profile catalog {source} schema mismatch")
    if document.get("revision") != expected_revision:
        failures.append(f"profile catalog {source} revision mismatch")
    if document.get("state") != "current":
        failures.append(f"profile catalog {source} state mismatch")
    if document.get("current_binary") != expected_current_binary:
        failures.append(f"profile catalog {source} current_binary mismatch")
    if document.get("current_assets") != expected_current_assets:
        failures.append(f"profile catalog {source} current_assets mismatch")
    actual_compatibility = document.get("compatibility")
    catalog_expected_compatibility = {
        **expected_compatibility,
        "requires_newer_binary": expected_requires_newer["binary"],
        "requires_newer_assets": expected_requires_newer["assets"],
    }
    for field, expected in catalog_expected_compatibility.items():
        actual = (
            actual_compatibility.get(field)
            if isinstance(actual_compatibility, dict)
            else None
        )
        if actual != expected:
            failures.append(f"profile catalog {source} compatibility {field} mismatch")
    return failures


def entries_by_url(entries: list[Any], failures: list[str], label: str) -> dict[str, dict[str, Any]]:
    by_url: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            failures.append(f"health evidence {label} entry is not an object")
            continue
        url = entry.get("url")
        if not isinstance(url, str):
            failures.append(f"health evidence {label} entry missing url")
            continue
        by_url[url] = entry
    return by_url


def fetch_and_verify_evidence_artifact(
    site: str,
    item: dict[str, Any],
    algorithm: str,
    label: str,
    expected_document: str | None = None,
) -> list[str]:
    url = item.get("url")
    if not isinstance(url, str):
        return [f"{label} missing url"]
    hash_key = "sha256" if algorithm == "sha256" else "hash"
    expected_hash = item.get(hash_key)
    if not isinstance(expected_hash, str):
        return [f"{label} {url} missing {hash_key}"]
    expected_size = item.get("size")
    if not isinstance(expected_size, int):
        return [f"{label} {url} missing size"]
    try:
        resolved_url = resolve_release_url(site, url)
    except ValueError as error:
        return [f"{label} {url}: {error}"]
    artifact = fetch_bytes(resolved_url)
    if artifact.error:
        return [artifact.error]
    if len(artifact.data) != expected_size:
        return [f"{label} {url} size mismatch"]
    if algorithm == "sha256":
        actual_hash = hashlib.sha256(artifact.data).hexdigest()
    elif algorithm == "blake3":
        if blake3 is None:
            return [f"{label} {url} cannot verify blake3 without Python dependency blake3"]
        actual_hash = blake3.blake3(artifact.data).hexdigest()
    else:
        return [f"{label} {url} unsupported hash algorithm {algorithm}"]
    if actual_hash != expected_hash:
        return [f"{label} {url} {algorithm} mismatch"]
    if expected_document is not None:
        content_failure = validate_evidence_document(artifact.data, expected_document, label, url)
        if content_failure is not None:
            return [content_failure]
    return []


def validate_evidence_document(
    artifact: bytes, expected_document: str, label: str, url: str
) -> str | None:
    try:
        document = json.loads(artifact)
    except json.JSONDecodeError as error:
        return f"{label} {url} invalid JSON: {error}"
    if not isinstance(document, dict):
        return f"{label} {url} document is not an object"
    if expected_document == "spdx":
        if document.get("spdxVersion") != "SPDX-2.3":
            return f"{label} {url} spdxVersion mismatch"
        return None
    if expected_document == "cyclonedx":
        if document.get("bomFormat") != "CycloneDX":
            return f"{label} {url} bomFormat mismatch"
        return None
    return f"{label} {url} unsupported evidence document {expected_document}"


def check_release_cache_headers(site: str, channel: str, health: dict[str, Any]) -> list[str]:
    site = site.rstrip("/")
    checks: list[tuple[str, str, tuple[str, ...]]] = [
        ("release index", f"{site}/", ("no-cache", "must-revalidate")),
        ("health JSON", f"{site}/health.json", ("no-cache", "must-revalidate")),
        (
            "channel manifest",
            f"{site}/assets/{channel}/manifest.json",
            ("no-cache", "must-revalidate"),
        ),
    ]

    assets = health.get("assets", {})
    if isinstance(assets, dict) and isinstance(assets.get("files"), list):
        for item in assets["files"]:
            if not isinstance(item, dict):
                continue
            url = item.get("url")
            if isinstance(url, str) and release_url_path(url).startswith("/assets/releases/"):
                checks.append(
                    (
                        "immutable asset",
                        resolve_release_url(site, url),
                        ("public", "max-age=31536000", "immutable"),
                    )
                )

    updates = health.get("updates", {})
    profiles = updates.get("profiles") if isinstance(updates, dict) else None
    profile_source = profiles.get("source") if isinstance(profiles, dict) else None
    if isinstance(profile_source, str) and release_url_path(profile_source).startswith(
        "/profiles/releases/"
    ):
        checks.append(
            (
                "immutable profile catalog",
                resolve_release_url(site, profile_source),
                ("public", "max-age=31536000", "immutable"),
            )
        )

    failures: list[str] = []
    for label, url, required_directives in checks:
        headers = fetch_headers(url)
        if headers.error:
            failures.append(headers.error)
            continue
        cache_control = headers.headers.get("cache-control", "")
        lower_cache_control = cache_control.lower()
        for directive in required_directives:
            if directive not in lower_cache_control:
                failures.append(f"{label} {url} Cache-Control must contain {directive}")
    return failures


def release_url_path(url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme in {"http", "https"}:
        return parsed.path
    return url


def resolve_release_url(site: str, url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme in {"http", "https"}:
        return url
    if url.startswith("/"):
        return f"{site.rstrip('/')}{url}"
    raise ValueError("evidence URL must be absolute or release-site relative")


def classic_protection_requires_pr_gate(data: Any) -> bool:
    if not isinstance(data, dict):
        return False
    required = data.get("required_status_checks")
    if not isinstance(required, dict):
        return False
    return required_checks_include_pr_gate(required.get("contexts")) or required_checks_include_pr_gate(
        required.get("checks")
    )


def active_branch_rules_require_pr_gate(data: Any) -> bool:
    if isinstance(data, dict) and data.get("enforcement") in {"evaluate", "disabled"}:
        return False
    rules = data.get("rules") if isinstance(data, dict) else data
    if not isinstance(rules, list):
        return False
    for rule in rules:
        if not isinstance(rule, dict):
            continue
        if rule.get("type") != "required_status_checks":
            continue
        parameters = rule.get("parameters")
        if not isinstance(parameters, dict):
            continue
        if required_checks_include_pr_gate(parameters.get("required_status_checks")):
            return True
        if required_checks_include_pr_gate(parameters.get("required_checks")):
            return True
    return False


def required_checks_include_pr_gate(checks: Any) -> bool:
    if not isinstance(checks, list):
        return False
    for check in checks:
        if check == "pr-gate":
            return True
        if isinstance(check, dict) and check.get("context") == "pr-gate":
            return True
    return False


@dataclass
class JsonResult:
    returncode: int
    data: Any | None
    stderr: str


@dataclass
class TextResult:
    returncode: int
    stdout: str
    stderr: str


@dataclass
class FetchText:
    text: str
    error: str | None = None


@dataclass
class FetchBytes:
    data: bytes
    error: str | None = None


@dataclass
class FetchJson:
    data: Any | None
    error: str | None = None


@dataclass
class FetchHeaders:
    headers: dict[str, str]
    error: str | None = None


def run_text(argv: list[str]) -> TextResult:
    completed = subprocess.run(argv, check=False, capture_output=True, text=True)
    return TextResult(completed.returncode, completed.stdout, completed.stderr)


def run_json(argv: list[str]) -> JsonResult:
    completed = subprocess.run(argv, check=False, capture_output=True, text=True)
    if completed.returncode != 0:
        return JsonResult(completed.returncode, None, completed.stderr)
    try:
        return JsonResult(0, json.loads(completed.stdout), completed.stderr)
    except json.JSONDecodeError as error:
        return JsonResult(1, None, f"invalid JSON from {' '.join(argv)}: {error}")


def fetch_text(url: str) -> FetchText:
    data = fetch_bytes(url)
    if data.error:
        return FetchText("", data.error)
    try:
        return FetchText(data.data.decode("utf-8"))
    except UnicodeDecodeError as error:
        return FetchText("", f"fetch {url}: {error}")


def fetch_bytes(url: str) -> FetchBytes:
    try:
        with urllib.request.urlopen(release_site_request(url), timeout=20) as response:
            return FetchBytes(response.read())
    except (OSError, urllib.error.URLError) as error:
        return FetchBytes(b"", f"fetch {url}: {error}")


def fetch_headers(url: str) -> FetchHeaders:
    try:
        request = release_site_request(url, method="HEAD")
        with urllib.request.urlopen(request, timeout=20) as response:
            return FetchHeaders({key.lower(): value for key, value in response.headers.items()})
    except (OSError, urllib.error.URLError) as error:
        return FetchHeaders({}, f"fetch headers {url}: {error}")


def release_site_request(url: str, *, method: str | None = None) -> urllib.request.Request:
    return urllib.request.Request(
        url,
        headers={"User-Agent": RELEASE_VALIDATOR_USER_AGENT},
        method=method,
    )


def fetch_json(url: str) -> FetchJson:
    text = fetch_text(url)
    if text.error:
        return FetchJson(None, text.error)
    try:
        return FetchJson(json.loads(text.text))
    except json.JSONDecodeError as error:
        return FetchJson(None, f"parse {url}: {error}")


if __name__ == "__main__":
    raise SystemExit(main())
