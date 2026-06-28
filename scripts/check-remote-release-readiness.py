#!/usr/bin/env python3
"""Read-only checks for live release rails before cutting or deploying."""

from __future__ import annotations

import argparse
import hashlib
import json
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

    if health_data.get("schema") != "capsem.assets_channel.health.v1":
        failures.append("health schema mismatch")
    if health_urls.get("manifest") != manifest_path:
        failures.append("health manifest URL mismatch")
    if health_urls.get("asset_base") != "/assets/releases":
        failures.append("health asset base mismatch")
    if manifest_data.get("format") != 2:
        failures.append("manifest format mismatch")

    current_binary = health_current.get("binary")
    current_assets = health_current.get("assets")
    profile_source = health_profiles.get("source")
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
    profile_update_source = (
        health_update_profiles.get("source") if isinstance(health_update_profiles, dict) else None
    )
    if health_urls.get("profile_catalog") != profile_source:
        failures.append("health profile catalog URL mismatch")
    if profile_update_source != profile_source:
        failures.append("health profile update source mismatch")
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
    if manifest_assets.get("current") != current_assets:
        failures.append("current asset mismatch between health and manifest")
    if manifest_binaries.get("current") != current_binary:
        failures.append("current binary mismatch between health and manifest")
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

    current_asset_release = next(
        (
            release
            for release in health_data.get("asset_releases", [])
            if isinstance(release, dict) and release.get("version") == current_assets
        ),
        {},
    )
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
        if url not in host_binary_by_url:
            failures.append(f"host SBOM evidence {url} missing from host binary files")
            continue
        failures.extend(
            fetch_and_verify_evidence_artifact(site, sbom, "sha256", "host SBOM evidence")
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
        failures.extend(fetch_and_verify_evidence_artifact(site, obom, "blake3", "VM OBOM evidence"))

    saw_host_sbom_attestation = False
    host_sbom_attestation_subjects: set[str] = set()
    for attestation in attestations:
        if not isinstance(attestation, dict):
            failures.append("health evidence attestations entry is not an object")
            continue
        attestation_name = attestation.get("name")
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
    site: str, item: dict[str, Any], algorithm: str, label: str
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
    return []


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
        with urllib.request.urlopen(url, timeout=20) as response:
            return FetchBytes(response.read())
    except (OSError, urllib.error.URLError) as error:
        return FetchBytes(b"", f"fetch {url}: {error}")


def fetch_headers(url: str) -> FetchHeaders:
    try:
        request = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(request, timeout=20) as response:
            return FetchHeaders({key.lower(): value for key, value in response.headers.items()})
    except (OSError, urllib.error.URLError) as error:
        return FetchHeaders({}, f"fetch headers {url}: {error}")


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
