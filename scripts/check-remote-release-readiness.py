#!/usr/bin/env python3
"""Read-only checks for live release rails before cutting or deploying."""

from __future__ import annotations

import argparse
import hashlib
import json
import socket
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlparse

import blake3


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
    if "needs: [test-linux, test, test-install]" not in text:
        return CheckResult(
            "remote ci.yaml pr-gate",
            False,
            "remote pr-gate does not aggregate test-linux, test, and test-install",
        )
    return CheckResult("remote ci.yaml pr-gate", True, "pr-gate aggregates required jobs")


def check_remote_branch_protection(repo: str, branch: str) -> CheckResult:
    classic = run_json(["gh", "api", f"repos/{repo}/branches/{branch}/protection"])
    classic_required = False
    classic_detail = ""
    if classic.returncode == 0 and classic.data is not None:
        classic_required = required_status_checks_include_pr_gate(classic.data)
        classic_detail = "classic branch protection"

    ruleset_required = False
    ruleset_details: list[str] = []
    rulesets = run_json(["gh", "api", f"repos/{repo}/rulesets"])
    if rulesets.returncode == 0 and isinstance(rulesets.data, list):
        for summary in rulesets.data:
            ruleset_id = summary.get("id") if isinstance(summary, dict) else None
            if ruleset_id is None:
                continue
            detail = run_json(["gh", "api", f"repos/{repo}/rulesets/{ruleset_id}"])
            if detail.returncode != 0 or detail.data is None:
                continue
            if required_status_checks_include_pr_gate(detail.data):
                ruleset_required = True
                ruleset_details.append(str(summary.get("name") or ruleset_id))

    if classic_required or ruleset_required:
        sources = []
        if classic_required:
            sources.append(classic_detail)
        sources.extend(f"ruleset {name}" for name in ruleset_details)
        return CheckResult(
            "remote branch protection requires pr-gate",
            True,
            ", ".join(sources),
        )

    detail = "pr-gate is not required by classic branch protection or active rulesets"
    if classic.returncode != 0:
        detail += f"; classic protection probe: {classic.stderr.strip()}"
    if rulesets.returncode != 0:
        detail += f"; ruleset probe: {rulesets.stderr.strip()}"
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
    if health_data.get("schema") != "capsem.assets_channel.health.v1":
        failures.append("health schema mismatch")
    if health_data.get("urls", {}).get("manifest") != manifest_path:
        failures.append("health manifest URL mismatch")
    if health_data.get("urls", {}).get("asset_base") != "/assets/releases":
        failures.append("health asset base mismatch")
    if manifest_data.get("format") != 2:
        failures.append("manifest format mismatch")

    current = health_data.get("current", {})
    current_binary = current.get("binary")
    current_assets = current.get("assets")
    if manifest_data.get("assets", {}).get("current") != current_assets:
        failures.append("current asset mismatch between health and manifest")
    if manifest_data.get("binaries", {}).get("current") != current_binary:
        failures.append("current binary mismatch between health and manifest")
    for label, value in (
        ("current binary", current_binary),
        ("current assets", current_assets),
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

    if failures:
        return CheckResult("release.capsem.org contract", False, "; ".join(failures))
    return CheckResult(
        "release.capsem.org contract",
        True,
        "index, health.json, manifest, and evidence artifacts agree",
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
    host_sbom_urls = {
        item["url"]
        for item in host_sboms
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

    for attestation in attestations:
        if not isinstance(attestation, dict):
            failures.append("health evidence attestations entry is not an object")
            continue
        predicate_type = attestation.get("predicate_type")
        if not isinstance(predicate_type, str) or not predicate_type:
            failures.append("health evidence attestation predicate_type missing")
        verify_command = attestation.get("verify_command")
        if not isinstance(verify_command, str) or "gh attestation verify" not in verify_command:
            failures.append("health evidence attestation verify_command must use gh attestation verify")
        predicate_url = attestation.get("predicate_url")
        if predicate_url is not None and predicate_url not in host_sbom_urls:
            failures.append(f"attestation predicate_url {predicate_url} missing from host SBOM evidence")
        subjects = attestation.get("subjects")
        if not isinstance(subjects, list) or not subjects:
            failures.append("health evidence attestation subjects missing")
            continue
        for subject in subjects:
            if not isinstance(subject, str):
                failures.append("health evidence attestation subject is not a string")
                continue
            if subject not in host_binary_by_url and subject not in asset_by_url:
                failures.append(f"attestation subject {subject} missing from published file lists")

    return failures


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
        actual_hash = blake3.blake3(artifact.data).hexdigest()
    else:
        return [f"{label} {url} unsupported hash algorithm {algorithm}"]
    if actual_hash != expected_hash:
        return [f"{label} {url} {algorithm} mismatch"]
    return []


def resolve_release_url(site: str, url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme in {"http", "https"}:
        return url
    if url.startswith("/"):
        return f"{site.rstrip('/')}{url}"
    raise ValueError("evidence URL must be absolute or release-site relative")


def required_status_checks_include_pr_gate(data: Any) -> bool:
    if isinstance(data, dict):
        for key, value in data.items():
            if key in {"context", "name"} and value == "pr-gate":
                return True
            if key == "required_status_checks" and required_status_checks_include_pr_gate(value):
                return True
            if key == "required_checks" and required_status_checks_include_pr_gate(value):
                return True
            if required_status_checks_include_pr_gate(value):
                return True
    elif isinstance(data, list):
        return any(required_status_checks_include_pr_gate(item) for item in data)
    elif isinstance(data, str):
        return data == "pr-gate"
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
