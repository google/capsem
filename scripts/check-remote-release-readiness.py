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


REQUIRED_PR_GATE_JOBS = (
    "test-linux",
    "test",
    "test-install",
    "docs-build",
    "site-build",
    "release-site-build",
)
REQUIRED_PR_GATE_RESULT_CHECKS = (
    ("test-linux", "TEST_LINUX_RESULT"),
    ("test", "TEST_MACOS_RESULT"),
    ("test-install", "TEST_INSTALL_RESULT"),
    ("docs-build", "DOCS_BUILD_RESULT"),
    ("site-build", "SITE_BUILD_RESULT"),
    ("release-site-build", "RELEASE_SITE_BUILD_RESULT"),
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
    channels_url = f"{site}/channels.json"

    index = fetch_text(index_url)
    if index.error:
        return CheckResult("release.capsem.org contract", False, index.error)
    channels = fetch_json(channels_url)
    if channels.error:
        return CheckResult("release.capsem.org contract", False, channels.error)

    failures: list[str] = []
    channels_data = channels.data if isinstance(channels.data, dict) else {}
    channel_entries = require_object(channels_data, "channels", "channels catalog", failures)
    channel_data = require_object(channel_entries, channel, f"channels.{channel}", failures)
    manifest_record = select_channel_manifest_record(channel_data, failures)
    manifest_path = manifest_record.get("url")
    if not isinstance(manifest_path, str):
        failures.append("channel manifest URL missing")
        manifest_path = f"/assets/{channel}/manifest.json"
    manifest_url = resolve_release_url(site, manifest_path)
    manifest_payload = fetch_bytes(manifest_url)
    if manifest_payload.error:
        return CheckResult("release.capsem.org contract", False, manifest_payload.error)
    try:
        manifest_data = json.loads(manifest_payload.data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        return CheckResult(
            "release.capsem.org contract",
            False,
            f"manifest JSON parse failed for {manifest_url}: {error}",
        )
    if not isinstance(manifest_data, dict):
        return CheckResult(
            "release.capsem.org contract",
            False,
            f"manifest {manifest_url} document is not an object",
        )
    if is_release_graph_manifest(manifest_data):
        failures.extend(
            check_release_graph_manifest_contract(
                site=site,
                channel=channel,
                index_text=index.text,
                channels_data=channels_data,
                channel_data=channel_data,
                manifest_record=manifest_record,
                manifest_path=manifest_path,
                manifest_payload=manifest_payload.data,
                manifest_data=manifest_data,
            )
        )
        if failures:
            return CheckResult("release.capsem.org contract", False, "; ".join(failures))
        return CheckResult(
            "release.capsem.org contract",
            True,
            "index, channels.json, graph manifest, profile catalog, profile artifacts, and cache headers agree",
        )

    if release_url_path(manifest_path) != f"/assets/{channel}/manifest.json":
        failures.append("channel manifest URL mismatch")

    manifest_assets = require_object(manifest_data, "assets", "manifest assets", failures)
    manifest_binaries = require_object(manifest_data, "binaries", "manifest binaries", failures)
    manifest_asset_releases = require_object(
        manifest_assets, "releases", "manifest asset releases", failures
    )
    manifest_binary_releases = require_object(
        manifest_binaries, "releases", "manifest binary releases", failures
    )

    if channels_data.get("version") != 1:
        failures.append("channels catalog version mismatch")
    if manifest_record.get("status") not in {"current", "supported", "deprecated"}:
        failures.append("channel selected manifest status is not selectable")
    digest = require_object(manifest_record, "digest", "channel manifest digest", failures)
    if blake3 is not None and isinstance(digest.get("blake3"), str):
        actual_manifest_hash = blake3.blake3(manifest_payload.data).hexdigest()
        if digest.get("blake3") != actual_manifest_hash:
            failures.append("channel manifest BLAKE3 mismatch")
    elif blake3 is None:
        failures.append("channel manifest cannot verify blake3 without Python dependency blake3")
    else:
        failures.append("channel manifest BLAKE3 missing")
    asset_base = manifest_record.get("asset_base") or manifest_data.get("asset_base")
    if not valid_asset_base(asset_base):
        failures.append("channel asset base mismatch")
    if manifest_data.get("format") != 2:
        failures.append("manifest format mismatch")

    current_binary = manifest_binaries.get("current")
    current_assets = manifest_assets.get("current")
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
    if manifest_record.get("binary_version") != current_binary:
        failures.append("channel binary version mismatch with manifest")
    if manifest_record.get("asset_version") != current_assets:
        failures.append("channel asset version mismatch with manifest")
    profile_catalog = require_object(
        channel_data, "profile_catalog", f"channels.{channel}.profile_catalog", failures
    )
    profile_source = profile_catalog.get("source")
    failures.extend(
        check_profile_catalog_summary(
            site,
            profile_source,
            profile_catalog.get("hash"),
            profile_catalog.get("revision"),
        )
    )
    expected_asset_files = current_asset_file_refs(
        asset_base,
        current_assets,
        current_manifest_asset_release,
        failures,
    )
    for item in expected_asset_files:
        failures.extend(fetch_and_verify_evidence_artifact(site, item, "blake3", "VM asset file"))
    expected_binary_files = current_binary_file_refs(
        current_binary,
        current_manifest_binary_release,
        failures,
    )
    if expected_binary_files:
        failures.append("manifest binary package files must move to package inventory contract")
    for label, value in (
        ("current binary", current_binary),
        ("current assets", current_assets),
        ("generated timestamp", channels_data.get("generated_at")),
        ("profile revision", profile_catalog.get("revision")),
        ("profile catalog", profile_source),
        ("channel manifest", manifest_path),
        ("channels catalog", "/channels.json"),
    ):
        if not isinstance(value, str):
            failures.append(f"release channel {label} missing")
        elif value not in index.text:
            failures.append(f"index missing {label} {value}")

    for version, manifest_release in manifest_asset_releases.items():
        if not isinstance(version, str) or not isinstance(manifest_release, dict):
            failures.append("manifest asset release entry malformed")
            continue

    current_asset_date = (
        current_manifest_asset_release.get("date")
        if isinstance(current_manifest_asset_release, dict)
        else None
    )
    if not isinstance(current_asset_date, str):
        failures.append("current asset release date missing")
    elif current_asset_date not in index.text:
        failures.append("index missing current asset release date")

    failures.extend(
        check_release_cache_headers(site, channel, profile_source, expected_asset_files)
    )

    if failures:
        return CheckResult("release.capsem.org contract", False, "; ".join(failures))
    return CheckResult(
        "release.capsem.org contract",
        True,
        "index, channels.json, manifest, profile catalog, assets, and cache headers agree",
    )


def is_release_graph_manifest(manifest_data: dict[str, Any]) -> bool:
    return (
        isinstance(manifest_data.get("packages"), list)
        and isinstance(manifest_data.get("binaries"), list)
        and isinstance(manifest_data.get("profiles"), dict)
    )


def check_release_graph_manifest_contract(
    *,
    site: str,
    channel: str,
    index_text: str,
    channels_data: dict[str, Any],
    channel_data: dict[str, Any],
    manifest_record: dict[str, Any],
    manifest_path: str,
    manifest_payload: bytes,
    manifest_data: dict[str, Any],
) -> list[str]:
    failures: list[str] = []
    if channels_data.get("version") != 1:
        failures.append("channels catalog version mismatch")
    if manifest_record.get("status") not in {"current", "supported", "deprecated"}:
        failures.append("channel selected manifest status is not selectable")

    digest = require_object(manifest_record, "digest", "channel manifest digest", failures)
    expected_sha256 = digest.get("sha256")
    if not isinstance(expected_sha256, str):
        failures.append("channel manifest SHA-256 missing")
    elif hashlib.sha256(manifest_payload).hexdigest() != expected_sha256:
        failures.append("channel manifest SHA-256 mismatch")
    if blake3 is None:
        failures.append("channel manifest cannot verify blake3 without Python dependency blake3")
    else:
        expected_blake3 = digest.get("blake3")
        if not isinstance(expected_blake3, str):
            failures.append("channel manifest BLAKE3 missing")
        elif blake3.blake3(manifest_payload).hexdigest() != expected_blake3:
            failures.append("channel manifest BLAKE3 mismatch")

    if manifest_data.get("version") != manifest_record.get("version"):
        failures.append("manifest version mismatch with channel record")

    packages = require_list(manifest_data, "packages", failures)
    binaries = require_list(manifest_data, "binaries", failures)
    profiles = require_object(manifest_data, "profiles", "manifest profiles", failures)
    if not packages:
        failures.append("manifest packages empty")
    if not binaries:
        failures.append("manifest binaries empty")
    if not profiles:
        failures.append("manifest profiles empty")

    for package in packages:
        failures.extend(check_release_graph_file_descriptor(package, "package"))
    for binary in binaries:
        failures.extend(check_release_graph_file_descriptor(binary, "binary"))
        if not isinstance(binary.get("sbom_component_ref"), str):
            failures.append(f"binary {binary.get('name', '<unknown>')} SBOM component missing")

    profile_catalog = require_object(
        channel_data, "profile_catalog", f"channels.{channel}.profile_catalog", failures
    )
    catalog_document = fetch_profile_catalog_document(
        site,
        profile_catalog.get("source"),
        profile_catalog.get("hash"),
        profile_catalog.get("revision"),
        failures,
    )
    if isinstance(catalog_document, dict):
        catalog_profiles = catalog_document.get("profiles")
        if isinstance(catalog_profiles, list):
            catalog_ids = sorted(
                profile.get("id")
                for profile in catalog_profiles
                if isinstance(profile, dict) and isinstance(profile.get("id"), str)
            )
            if catalog_ids != sorted(profiles):
                failures.append("profile catalog ids mismatch with manifest profiles")

    channel_page = fetch_text(f"{site}/channels/{channel}/")
    if channel_page.error:
        failures.append(channel_page.error)
    for label, value in (
        ("manifest version", manifest_record.get("version")),
        ("channel manifest", manifest_path),
    ):
        if not isinstance(value, str):
            failures.append(f"release channel {label} missing")
        elif value not in index_text and (channel_page.error or value not in channel_page.text):
            failures.append(f"release pages missing {label} {value}")

    for profile_id, profile in profiles.items():
        if not isinstance(profile_id, str) or not isinstance(profile, dict):
            failures.append("manifest profile entry malformed")
            continue
        failures.extend(
            check_release_graph_profile(site, channel, profile_id, profile)
        )

    failures.extend(
        check_release_graph_cache_headers(
            site=site,
            manifest_path=manifest_path,
            profile_source=profile_catalog.get("source"),
            profiles=profiles,
        )
    )
    return failures


def check_release_graph_file_descriptor(item: Any, label: str) -> list[str]:
    if not isinstance(item, dict):
        return [f"{label} entry is not an object"]
    failures: list[str] = []
    name = item.get("name")
    version = item.get("version")
    digest = item.get("digest")
    if not isinstance(name, str):
        failures.append(f"{label} name missing")
    if not isinstance(version, str):
        failures.append(f"{label} version missing")
    if not isinstance(digest, dict):
        failures.append(f"{label} digest missing")
        return failures
    for key in ("sha256", "blake3", "hmac"):
        if not isinstance(digest.get(key), str):
            failures.append(f"{label} {name or '<unknown>'} digest {key} missing")
    return failures


def fetch_profile_catalog_document(
    site: str,
    source: Any,
    expected_hash: Any,
    expected_revision: Any,
    failures: list[str],
) -> dict[str, Any] | None:
    summary_failures = check_profile_catalog_summary(
        site, source, expected_hash, expected_revision
    )
    failures.extend(summary_failures)
    if summary_failures or not isinstance(source, str):
        return None
    catalog = fetch_json(resolve_release_url(site, source))
    if catalog.error:
        failures.append(catalog.error)
        return None
    if not isinstance(catalog.data, dict):
        failures.append(f"profile catalog {source} document is not an object")
        return None
    return catalog.data


def check_release_graph_profile(
    site: str,
    channel: str,
    profile_id: str,
    profile: dict[str, Any],
) -> list[str]:
    failures: list[str] = []
    profile_page = fetch_text(f"{site}/channels/{channel}/profiles/{profile_id}/")
    if profile_page.error:
        failures.append(profile_page.error)
        page_text = ""
    else:
        page_text = profile_page.text

    for field in ("id", "revision", "min_capsem_version"):
        if not isinstance(profile.get(field), str):
            failures.append(f"profile {profile_id} {field} missing")
    if profile.get("id") != profile_id:
        failures.append(f"profile {profile_id} id mismatch")

    config_entries = require_list(profile, "config", failures)
    images = require_list(profile, "images", failures)
    if not config_entries:
        failures.append(f"profile {profile_id} config empty")
    if not images:
        failures.append(f"profile {profile_id} images empty")

    for value in (profile.get("revision"), profile.get("name"), profile.get("id")):
        if isinstance(value, str) and page_text and value not in page_text:
            failures.append(f"profile page {profile_id} missing {value}")

    for item in config_entries:
        failures.extend(
            check_release_graph_artifact(site, item, f"profile {profile_id} config", page_text)
        )
    for image in images:
        if not isinstance(image, dict):
            failures.append(f"profile {profile_id} image entry is not an object")
            continue
        if not isinstance(image.get("architecture"), str):
            failures.append(f"profile {profile_id} image architecture missing")
        for artifact in require_list(image, "artifacts", failures):
            failures.extend(
                check_release_graph_artifact(
                    site, artifact, f"profile {profile_id} image artifact", page_text
                )
            )
        for evidence in require_list(image, "evidence", failures):
            failures.extend(
                check_release_graph_artifact(
                    site,
                    evidence,
                    f"profile {profile_id} evidence",
                    page_text,
                    expected_document="cyclonedx",
                )
            )
    return failures


def check_release_graph_artifact(
    site: str,
    item: Any,
    label: str,
    page_text: str,
    *,
    expected_document: str | None = None,
) -> list[str]:
    if not isinstance(item, dict):
        return [f"{label} entry is not an object"]
    failures: list[str] = []
    url = item.get("url")
    if not isinstance(url, str):
        return [f"{label} URL missing"]
    if "file://" in url:
        failures.append(f"{label} {url} must not use file://")
    if url.startswith("/profiles/releases/") is False:
        failures.append(f"{label} {url} must be a profile release artifact")
    digest = item.get("digest")
    if not isinstance(digest, dict):
        return failures + [f"{label} {url} digest missing"]
    for key in ("sha256", "blake3", "hmac"):
        value = digest.get(key)
        if not isinstance(value, str):
            failures.append(f"{label} {url} {key} missing")
        elif page_text and value not in page_text:
            failures.append(f"profile page missing {label} {key} for {url}")
    expected_bytes = item.get("bytes")
    if not isinstance(expected_bytes, int):
        failures.append(f"{label} {url} bytes missing")

    try:
        resolved_url = resolve_release_url(site, url)
    except ValueError as error:
        return failures + [f"{label} {url}: {error}"]
    artifact = fetch_bytes(resolved_url)
    if artifact.error:
        return failures + [artifact.error]
    if isinstance(expected_bytes, int) and len(artifact.data) != expected_bytes:
        failures.append(f"{label} {url} size mismatch")
    expected_sha256 = digest.get("sha256")
    if isinstance(expected_sha256, str) and hashlib.sha256(artifact.data).hexdigest() != expected_sha256:
        failures.append(f"{label} {url} sha256 mismatch")
    expected_blake3 = digest.get("blake3")
    if blake3 is None:
        failures.append(f"{label} {url} cannot verify blake3 without Python dependency blake3")
    elif isinstance(expected_blake3, str) and blake3.blake3(artifact.data).hexdigest() != expected_blake3:
        failures.append(f"{label} {url} blake3 mismatch")
    if expected_document is not None:
        content_failure = validate_evidence_document(
            artifact.data, expected_document, label, url
        )
        if content_failure is not None:
            failures.append(content_failure)
    return failures


def check_release_graph_cache_headers(
    *,
    site: str,
    manifest_path: str,
    profile_source: Any,
    profiles: dict[str, Any],
) -> list[str]:
    checks: list[tuple[str, str, tuple[str, ...]]] = [
        ("release index", f"{site}/", ("no-cache", "must-revalidate")),
        ("channels JSON", f"{site}/channels.json", ("no-cache", "must-revalidate")),
        (
            "channel manifest",
            resolve_release_url(site, manifest_path),
            ("no-cache", "must-revalidate"),
        ),
    ]
    if isinstance(profile_source, str):
        checks.append(
            (
                "immutable profile catalog",
                resolve_release_url(site, profile_source),
                ("public", "max-age=31536000", "immutable"),
            )
        )
    for profile in profiles.values():
        if not isinstance(profile, dict):
            continue
        for item in profile.get("config", []):
            add_release_artifact_cache_check(site, checks, item)
        for image in profile.get("images", []):
            if not isinstance(image, dict):
                continue
            for artifact in image.get("artifacts", []):
                add_release_artifact_cache_check(site, checks, artifact)
            for evidence in image.get("evidence", []):
                add_release_artifact_cache_check(site, checks, evidence)

    failures: list[str] = []
    for label, url, required_directives in checks:
        headers = fetch_headers(url)
        if headers.error:
            failures.append(headers.error)
            continue
        lower_cache_control = headers.headers.get("cache-control", "").lower()
        for directive in required_directives:
            if directive not in lower_cache_control:
                failures.append(f"{label} {url} Cache-Control must contain {directive}")
    return failures


def add_release_artifact_cache_check(
    site: str,
    checks: list[tuple[str, str, tuple[str, ...]]],
    item: Any,
) -> None:
    if not isinstance(item, dict):
        return
    url = item.get("url")
    if isinstance(url, str) and url.startswith("/profiles/releases/"):
        checks.append(
            (
                "immutable profile artifact",
                resolve_release_url(site, url),
                ("public", "max-age=31536000", "immutable"),
            )
        )


def select_channel_manifest_record(
    channel_data: dict[str, Any], failures: list[str]
) -> dict[str, Any]:
    manifests = channel_data.get("manifests")
    if not isinstance(manifests, list):
        failures.append("channel manifests missing or not a list")
        return {}
    by_status: dict[str, dict[str, Any]] = {}
    for item in manifests:
        if not isinstance(item, dict):
            failures.append("channel manifest entry is not an object")
            continue
        status = item.get("status")
        version = item.get("version")
        if status not in {"current", "supported", "deprecated", "revoked"}:
            failures.append(f"channel manifest {version} status mismatch")
            continue
        if status != "revoked" and status not in by_status:
            by_status[status] = item
    for status in ("current", "supported", "deprecated"):
        if status in by_status:
            return by_status[status]
    failures.append("channel has no selectable manifest")
    return {}


def check_profile_catalog_summary(
    site: str,
    source: Any,
    expected_hash: Any,
    expected_revision: Any,
) -> list[str]:
    if not isinstance(source, str):
        return ["profile catalog source missing or not a string"]
    if not source.startswith("/profiles/releases/") or not source.endswith("/catalog.json"):
        return ["profile catalog source must be a release-channel artifact path"]
    if not isinstance(expected_hash, str):
        return [f"profile catalog {source} hash missing or not a string"]
    if not isinstance(expected_revision, str):
        return [f"profile catalog {source} revision missing or not a string"]

    catalog = fetch_bytes(resolve_release_url(site, source))
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
        return failures + [f"profile catalog {source} document is not an object"]
    if document.get("schema") != "capsem.profile_catalog.v1":
        failures.append(f"profile catalog {source} schema mismatch")
    if document.get("revision") != expected_revision:
        failures.append(f"profile catalog {source} revision mismatch")
    if not isinstance(document.get("profiles"), list):
        failures.append(f"profile catalog {source} profiles missing or not a list")
    return failures


def check_release_evidence(site: str, release_data: dict[str, Any]) -> list[str]:
    evidence = release_data.get("evidence")
    if not isinstance(evidence, dict):
        return ["health evidence missing"]

    failures: list[str] = []
    vm_oboms = require_list(evidence, "vm_oboms", failures)
    host_sboms = require_list(evidence, "host_sboms", failures)
    host_binary_files = require_list(evidence, "host_binary_files", failures)
    attestations = require_list(evidence, "attestations", failures)
    asset_files = require_list(release_data.get("assets", {}), "files", failures)

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
    asset_base: Any,
    asset_version: Any,
    release: Any,
    failures: list[str],
) -> list[dict[str, Any]]:
    if not valid_asset_base(asset_base):
        return []
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
            url = asset_url_from_base(asset_base, asset_version, arch, logical_name)
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
        blake3_hash = item.get("blake3")
        size = item.get("size")
        if not isinstance(name, str):
            failures.append("manifest binary file name missing")
            continue
        url = f"{release_base}/{name}"
        if not isinstance(sha256, str):
            failures.append(f"manifest binary file {url} sha256 missing")
            continue
        if not isinstance(blake3_hash, str):
            failures.append(f"manifest binary file {url} blake3 missing")
            continue
        if not isinstance(size, int):
            failures.append(f"manifest binary file {url} size missing")
            continue
        refs.append(
            {"name": name, "url": url, "sha256": sha256, "blake3": blake3_hash, "size": size}
        )
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
        for field in ("name", "sha256", "blake3", "size"):
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
        files = document.get("files")
        if files is not None:
            if not isinstance(files, list):
                return f"{label} {url} SPDX files must be an array"
            for file in files:
                if not isinstance(file, dict):
                    return f"{label} {url} SPDX files entry is not an object"
                spdx_id = file.get("SPDXID", "<unknown>")
                checksums = file.get("checksums")
                if not isinstance(checksums, list):
                    return f"{label} {url} SPDX file {spdx_id} missing SHA256 checksum"
                has_sha256 = False
                for checksum in checksums:
                    if not isinstance(checksum, dict):
                        continue
                    algorithm = checksum.get("algorithm")
                    checksum_value = checksum.get("checksumValue")
                    if (
                        isinstance(algorithm, str)
                        and algorithm.upper() == "SHA256"
                        and isinstance(checksum_value, str)
                        and len(checksum_value) == 64
                        and all(ch in "0123456789abcdefABCDEF" for ch in checksum_value)
                    ):
                        has_sha256 = True
                        break
                if not has_sha256:
                    return f"{label} {url} SPDX file {spdx_id} missing SHA256 checksum"
        return None
    if expected_document == "cyclonedx":
        if document.get("bomFormat") != "CycloneDX":
            return f"{label} {url} bomFormat mismatch"
        return None
    return f"{label} {url} unsupported evidence document {expected_document}"


def check_release_cache_headers(
    site: str,
    channel: str,
    profile_source: Any,
    asset_files: list[dict[str, Any]],
) -> list[str]:
    site = site.rstrip("/")
    checks: list[tuple[str, str, tuple[str, ...]]] = [
        ("release index", f"{site}/", ("no-cache", "must-revalidate")),
        ("channels JSON", f"{site}/channels.json", ("no-cache", "must-revalidate")),
        (
            "channel manifest",
            f"{site}/assets/{channel}/manifest.json",
            ("no-cache", "must-revalidate"),
        ),
    ]

    for item in asset_files:
        url = item.get("url")
        if (
            isinstance(url, str)
            and not urlparse(url).scheme
            and release_url_path(url).startswith("/assets/releases/")
        ):
            checks.append(
                (
                    "immutable asset",
                    resolve_release_url(site, url),
                    ("public", "max-age=31536000", "immutable"),
                )
            )

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


def valid_asset_base(asset_base: Any) -> bool:
    if not isinstance(asset_base, str) or not asset_base:
        return False
    parsed = urlparse(asset_base)
    return asset_base.startswith("/") or parsed.scheme in {"http", "https"}


def asset_url_from_base(asset_base: str, asset_version: str, arch: str, logical_name: str) -> str:
    asset_base = asset_base.rstrip("/")
    if "{asset_version}" in asset_base:
        version_base = asset_base.replace("{asset_version}", asset_version)
    else:
        version_base = f"{asset_base}/{asset_version}"
    return f"{version_base.rstrip('/')}/{arch}-{logical_name}"


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
    except urllib.error.HTTPError as error:
        error.close()
        return FetchBytes(b"", f"fetch {url}: {error}")
    except (OSError, urllib.error.URLError) as error:
        return FetchBytes(b"", f"fetch {url}: {error}")


def fetch_headers(url: str) -> FetchHeaders:
    try:
        request = release_site_request(url, method="HEAD")
        with urllib.request.urlopen(request, timeout=20) as response:
            return FetchHeaders({key.lower(): value for key, value in response.headers.items()})
    except urllib.error.HTTPError as error:
        error.close()
        return FetchHeaders({}, f"fetch headers {url}: {error}")
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
