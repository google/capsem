"""Local release-channel contract tests.

These tests build the generated release-channel dist with capsem-admin, serve it
with Cloudflare Pages _headers semantics, and run the public release-site
validator against the local URL.
"""

from __future__ import annotations

import contextlib
import functools
import hashlib
import http.server
import importlib.util
import io
import json
import os
import shutil
import socketserver
import subprocess
import sys
import tarfile
import threading
from collections.abc import Iterator
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from urllib.parse import urlparse

import pytest
from helpers.release_site import build_release_channel_site

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CHANNEL = "stable"
BUILD_COMPLETE_SPEC = importlib.util.spec_from_file_location(
    "build_complete_release_channel",
    PROJECT_ROOT / "scripts" / "build-complete-release-channel.py",
)
assert BUILD_COMPLETE_SPEC is not None and BUILD_COMPLETE_SPEC.loader is not None
BUILD_COMPLETE = importlib.util.module_from_spec(BUILD_COMPLETE_SPEC)
BUILD_COMPLETE_SPEC.loader.exec_module(BUILD_COMPLETE)
DEPLOY_FRESHNESS_SPEC = importlib.util.spec_from_file_location(
    "check_channel_deploy_freshness",
    PROJECT_ROOT / "scripts" / "check-channel-deploy-freshness.py",
)
assert DEPLOY_FRESHNESS_SPEC is not None and DEPLOY_FRESHNESS_SPEC.loader is not None
DEPLOY_FRESHNESS = importlib.util.module_from_spec(DEPLOY_FRESHNESS_SPEC)
DEPLOY_FRESHNESS_SPEC.loader.exec_module(DEPLOY_FRESHNESS)
SNAPSHOT_SPEC = importlib.util.spec_from_file_location(
    "release_site_snapshot",
    PROJECT_ROOT / "scripts" / "release_site_snapshot.py",
)
assert SNAPSHOT_SPEC is not None and SNAPSHOT_SPEC.loader is not None
SNAPSHOT = importlib.util.module_from_spec(SNAPSHOT_SPEC)
SNAPSHOT_SPEC.loader.exec_module(SNAPSHOT)
ROLLBACK_SPEC = importlib.util.spec_from_file_location(
    "cloudflare_pages_rollback",
    PROJECT_ROOT / "scripts" / "cloudflare_pages_rollback.py",
)
assert ROLLBACK_SPEC is not None and ROLLBACK_SPEC.loader is not None
ROLLBACK = importlib.util.module_from_spec(ROLLBACK_SPEC)
ROLLBACK_SPEC.loader.exec_module(ROLLBACK)
pytestmark = pytest.mark.build_chain


def test_deploy_workflow_preview_proves_exact_bytes_and_restores_prior_production() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-channel.yaml").read_text(
        encoding="utf-8"
    )

    assert "validate_complete_public_channels:" in workflow
    assert "default: true" in workflow
    assert "scripts/check-release-site-contract.py" in workflow
    assert workflow.count("CHANNEL_ARGS=(--catalog-members)") == 2
    assert "--dist-verification-only" in workflow
    assert "CHANNEL_ARGS=(--channel stable --channel nightly)" not in workflow
    assert '--base-url "$RELEASE_SITE_URL"' in workflow
    assert "--attempts 30" in workflow
    assert "group: capsem-public-channel-deploy" in workflow
    assert "cancel-in-progress: false" in workflow
    assert (
        "- name: Prove untouched public channels remain unchanged\n"
        "        if: ${{ inputs.activate_production }}"
    ) in workflow
    freshness = workflow.index("check-channel-deploy-freshness.py")
    capture = workflow.index("      - name: Capture current production deployment")
    prior_snapshot = workflow.index("      - name: Snapshot current production distribution")
    preview = workflow.index("      - name: Deploy immutable preview")
    preview_check = workflow.index("      - name: Validate preview distribution")
    activation = workflow.index("      - name: Activate verified production distribution")
    activation_check = workflow.index("      - name: Validate activated production bytes")
    decision = workflow.index("      - name: Decide production recovery")
    rollback = workflow.index("      - name: Restore prior production deployment")
    rollback_check = workflow.index("      - name: Verify restored production bytes")
    verdict = workflow.index("      - name: Require successful production activation")
    assert (
        freshness
        < capture
        < prior_snapshot
        < preview
        < preview_check
        < activation
        < activation_check
        < decision
        < rollback
        < rollback_check
        < verdict
    )
    assert "activate_production:" in workflow
    assert "workflow_dispatch:" not in workflow
    assert "artifact_run_id:" in workflow
    assert "scripts/verify-release-recovery-run.py" in workflow
    assert 'test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)"' in workflow
    assert "github-token: ${{ github.token }}" in workflow
    assert "run-id: ${{ inputs.artifact_run_id }}" in workflow
    assert "format('capsem-preview-{0}-{1}', github.run_id, github.run_attempt)" in workflow
    assert "PREVIEW_URL: ${{ steps.preview.outputs.deployment-url }}" in workflow
    assert "PREVIEW_URL: ${{ inputs.release_site_url }}" not in workflow
    assert "inputs.activate_production && steps.preview.outputs.deployment-url" not in workflow
    assert "--snapshot-out target/release-channel-deployment/candidate-release.json" in workflow
    assert "--expect-snapshot target/release-channel-deployment/candidate-release.json" in workflow
    assert "--expect-snapshot target/release-channel-deployment/prior-release.json" in workflow
    prior_step = workflow[prior_snapshot:preview]
    rollback_step = workflow[rollback_check:verdict]
    assert "--snapshot-only" in prior_step
    assert "--snapshot-only" in rollback_step
    preview_step = workflow[preview_check:activation]
    activation_step = workflow[activation_check:decision]
    assert '--dist "$DIST_DIR"' in preview_step
    assert "--dist-verification-only" in preview_step
    assert '--dist "$DIST_DIR"' not in activation_step
    assert "--snapshot-only" not in preview_step
    assert "--snapshot-only" not in activation_step
    assert (
        "PRODUCTION_DEPLOYMENT_ID: ${{ steps.production.outputs.pages-deployment-id }}" in workflow
    )
    assert '--deployment-id "$PRODUCTION_DEPLOYMENT_ID"' in workflow
    assert "steps.recovery.outputs.restore == 'true'" in workflow
    assert "continue-on-error: true" in workflow[activation:decision]
    assert "steps.production.outcome != 'skipped'" in workflow[verdict:]
    assert "cloudflare_cache_purge.py" not in workflow

    staging = (PROJECT_ROOT / ".github" / "workflows" / "release-channel-staging.yaml").read_text(
        encoding="utf-8"
    )
    assert "activate_production: false" in staging
    assert "astral-sh/setup-uv@d4b2f3b6ecc6e67c4457f6d3e41ec42d3d0fcb86" in staging
    assert "uv sync --frozen" in staging
    assert "bash scripts/rehearse-asset-channel-staging.sh" in staging
    assert "scripts/write-release-site-ci-fixture.py" not in staging
    assert "--without-binary-files" not in staging


def test_asset_staging_rehearsal_builds_a_complete_public_shape(tmp_path: Path) -> None:
    fixture = tmp_path / "fixture"
    dist = tmp_path / "dist"
    evidence = tmp_path / "evidence"
    command = [
        "bash",
        "scripts/rehearse-asset-channel-staging.sh",
        "staging",
        "1.0.2",
        str(fixture),
        str(dist),
        str(evidence),
    ]
    _run(command)

    manifest = json.loads((dist / "assets" / "staging" / "manifest.json").read_text())
    assert manifest["packages"]
    assert manifest["profiles"]
    assert (evidence / "candidate-release.json").is_file()

    replay = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert replay.returncode != 0
    assert "refusing stale asset staging path" in replay.stderr


def test_staging_workflows_keep_mutable_outputs_outside_cargo_cache() -> None:
    workflows = {
        name: (PROJECT_ROOT / ".github" / "workflows" / name).read_text(encoding="utf-8")
        for name in ("release-channel-staging.yaml", "release-binary-staging.yaml")
    }

    for name, workflow in workflows.items():
        assert "Swatinem/rust-cache" in workflow
        assert "$RUNNER_TEMP/" in workflow
        assert "target/release-channel" not in workflow, f"{name} stages into Cargo's cache"

    binary = workflows["release-binary-staging.yaml"]
    assert "target/binary-staging-packages" not in binary
    assert "target/binary-channel-dry-run" not in binary


def test_binary_staging_builds_parseable_packages_with_production_sbom() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "release-binary-staging.yaml").read_text(
        encoding="utf-8"
    )

    assert "scripts/write-binary-staging-artifacts.sh" in workflow
    assert "scripts/fetch-channel-source-manifest.py" in workflow
    assert "scripts/build-complete-release-channel.py" in workflow
    assert "uv sync --frozen" in workflow
    assert 'curl -fsSL "$ASSET_MANIFEST_URL"' not in workflow
    assert '--primary-channel "$ASSET_CHANNEL"' in workflow
    assert "inputs.channel" not in workflow
    assert "dpkg-deb --build" not in workflow
    assert "dry-run deb" not in workflow
    assert "capsem-binary-dry-run" not in workflow


def test_binary_staging_proof_rejects_vm_asset_drift(tmp_path: Path) -> None:
    root = tmp_path / "binary-channel"
    root.mkdir()
    before = {
        "profiles": {"code": {"revision": "1.2.3"}},
        "packages": [{"version": "1.3.0"}],
    }
    after = {
        "profiles": before["profiles"],
        "packages": [{"version": "1.4.0"}],
    }
    (root / "manifest.before.json").write_text(json.dumps(before), encoding="utf-8")
    (root / "manifest.json").write_text(json.dumps(after), encoding="utf-8")
    command = [
        sys.executable,
        "scripts/write-binary-channel-staging-proof.py",
        str(root),
    ]
    _run(command)

    proof = json.loads((root / "proof.json").read_text(encoding="utf-8"))
    assert proof["vm_assets_unchanged"] is True
    assert proof["binary_version"] == "1.4.0"
    assert proof["asset_version"] == "1.2.3"

    after["profiles"]["code"]["revision"] = "9.9.9"
    (root / "manifest.json").write_text(json.dumps(after), encoding="utf-8")
    drift = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert drift.returncode != 0
    assert "binary dry-run changed profile image metadata" in drift.stderr

    legacy_before = {
        "assets": {"current": "1.2.3"},
        "binaries": {"current": "1.3.0"},
    }
    legacy_after = {
        "assets": legacy_before["assets"],
        "binaries": {"current": "1.4.0"},
    }
    (root / "manifest.before.json").write_text(json.dumps(legacy_before), encoding="utf-8")
    (root / "manifest.json").write_text(json.dumps(legacy_after), encoding="utf-8")
    _run(command)
    legacy_proof = json.loads((root / "proof.json").read_text(encoding="utf-8"))
    assert legacy_proof["binary_version"] == "1.4.0"
    assert legacy_proof["asset_version"] == "1.2.3"

    legacy_after["assets"]["current"] = "9.9.9"
    (root / "manifest.json").write_text(json.dumps(legacy_after), encoding="utf-8")
    legacy_drift = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert legacy_drift.returncode != 0
    assert "binary dry-run changed VM asset metadata" in legacy_drift.stderr


def test_binary_staging_artifacts_are_deterministic_and_recordable(tmp_path: Path) -> None:
    version = "1.4.9999999999"
    runs = []
    for name, umask in (("first", "022"), ("second", "077")):
        root = tmp_path / name
        artifacts = root / "artifacts"
        _run(
            [
                "bash",
                "-c",
                'umask "$1"; shift; exec "$@"',
                "binary-staging",
                umask,
                "bash",
                "scripts/write-binary-staging-artifacts.sh",
                version,
                str(artifacts),
                str(root / "work"),
            ]
        )
        runs.append(artifacts)

    stale = subprocess.run(
        [
            "bash",
            "scripts/write-binary-staging-artifacts.sh",
            version,
            str(runs[0]),
            str(tmp_path / "first" / "work"),
        ],
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert stale.returncode == 1
    assert "refusing stale binary staging path" in stale.stderr

    artifact_names = {
        f"Capsem-{version}.pkg",
        f"Capsem_{version}_arm64.deb",
        "capsem-sbom.spdx.json",
    }
    assert {path.name for path in runs[0].iterdir()} == artifact_names
    assert {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in runs[0].iterdir()
    } == {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in runs[1].iterdir()}

    fixture = json.loads(
        (
            PROJECT_ROOT
            / "tests"
            / "capsem-release"
            / "fixtures"
            / "release-graph-stable-nightly.json"
        ).read_text(encoding="utf-8")
    )
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps(fixture["manifests"]["stable"]["1.0.2"], indent=2) + "\n",
        encoding="utf-8",
    )
    fake_bin = tmp_path / "fake-bin"
    fake_bin.mkdir()
    fake_pkgutil = fake_bin / "pkgutil"
    fake_pkgutil.write_text(
        "#!/bin/sh\necho 'synthetic package reached pkgutil' >&2\nexit 99\n",
        encoding="utf-8",
    )
    fake_pkgutil.chmod(0o755)
    _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest),
        "--version",
        version,
        "--source-commit",
        "0" * 40,
        *(
            argument
            for name in sorted(artifact_names)
            for argument in ("--artifact", str(runs[0] / name))
        ),
        env={"PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}"},
    )
    packages = json.loads(manifest.read_text(encoding="utf-8"))["packages"]
    assert {package["kind"] for package in packages} == {"debian_package", "macos_pkg"}
    assert {binary["name"] for package in packages for binary in package["binaries"]} == {
        "capsem-app",
        "capsem-tray",
    }

    dist = tmp_path / "dist"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        manifest.resolve().as_uri(),
        "--asset-source-base",
        "https://github.example.test/assets-v{asset_version}",
        "--channel",
        "stable",
        "--manifest-version",
        "1.0.2",
        "--out-dir",
        str(dist),
    )
    built = json.loads((dist / "assets" / "stable" / "manifest.json").read_text())
    assert built["channel"] == "stable"
    assert {package["kind"] for package in built["packages"]} == {
        "debian_package",
        "macos_pkg",
    }


@pytest.mark.parametrize(
    ("production", "validation", "restore", "success"),
    [
        ("skipped", "skipped", False, False),
        ("failure", "skipped", True, False),
        ("failure", "failure", True, False),
        ("success", "skipped", True, False),
        ("success", "failure", True, False),
        ("success", "success", False, True),
    ],
)
def test_activation_failure_at_every_edge_has_one_safe_decision(
    production: str,
    validation: str,
    restore: bool,
    success: bool,
) -> None:
    assert ROLLBACK.activation_decision(production, validation) == {
        "restore": restore,
        "activation_success": success,
    }


def test_activation_decision_rejects_unknown_workflow_state() -> None:
    with pytest.raises(ROLLBACK.RollbackError, match="unsupported production"):
        ROLLBACK.activation_decision("cancelled", "skipped")


def _snapshot_checker(site: str, *, manifest: bytes = b'{"version":"1"}\n') -> Any:
    site = site.rstrip("/")
    return SimpleNamespace(
        _FETCH_BYTES_CACHE={
            f"{site}/channels.json": SimpleNamespace(
                data=b'{"channels":{"stable":{}}}\n', error=None
            ),
            f"{site}/assets/stable/manifest.json": SimpleNamespace(data=manifest, error=None),
            "https://github.example.test/release/immutable.pkg": SimpleNamespace(
                data=b"immutable package", error=None
            ),
        }
    )


def test_release_snapshot_is_location_independent_and_includes_external_artifacts() -> None:
    preview = SNAPSHOT.release_fetch_snapshot(
        _snapshot_checker("https://preview.release.pages.dev"),
        "https://preview.release.pages.dev",
    )
    production = SNAPSHOT.release_fetch_snapshot(
        _snapshot_checker("https://release.capsem.org"),
        "https://release.capsem.org",
    )

    assert preview == production
    assert set(preview["entries"]) == {
        "/assets/stable/manifest.json",
        "/channels.json",
        "https://github.example.test/release/immutable.pkg",
    }


@pytest.mark.parametrize(
    "manifest",
    [b'{"version":"2"}\n', b"tampered manifest", b""],
)
def test_release_snapshot_rejects_any_changed_served_byte(tmp_path: Path, manifest: bytes) -> None:
    expected = SNAPSHOT.release_fetch_snapshot(
        _snapshot_checker("https://preview.release.pages.dev"),
        "https://preview.release.pages.dev",
    )
    snapshot_path = tmp_path / "candidate.json"
    SNAPSHOT.write_snapshot(snapshot_path, expected)
    actual = SNAPSHOT.release_fetch_snapshot(
        _snapshot_checker("https://release.capsem.org", manifest=manifest),
        "https://release.capsem.org",
    )

    with pytest.raises(RuntimeError, match=r"changed=.*manifest\.json"):
        SNAPSHOT.require_snapshot(snapshot_path, actual)


def test_release_snapshot_requires_catalog_and_manifest_evidence() -> None:
    checker = SimpleNamespace(
        _FETCH_BYTES_CACHE={
            "https://release.capsem.org/health.json": SimpleNamespace(data=b"{}", error=None)
        }
    )
    with pytest.raises(RuntimeError, match=r"channels\.json"):
        SNAPSHOT.release_fetch_snapshot(checker, "https://release.capsem.org")


def test_prior_distribution_snapshot_ignores_broken_external_references(tmp_path: Path) -> None:
    site = "https://release.capsem.org"
    checker = SimpleNamespace(_FETCH_BYTES_CACHE={})

    def failed_contract(**_kwargs: object) -> int:
        checker._FETCH_BYTES_CACHE.update(
            {
                f"{site}/channels.json": SimpleNamespace(data=b"{}\n", error=None),
                f"{site}/assets/stable/manifest.json": SimpleNamespace(
                    data=b'{"channel":"stable"}\n', error=None
                ),
                "https://github.example.test/missing.img": SimpleNamespace(
                    data=None, error="HTTP 404"
                ),
            }
        )
        return 1

    snapshot_path = tmp_path / "prior.json"

    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=failed_contract,
        attempts=1,
        delay_seconds=0,
        snapshot_out=snapshot_path,
        expect_snapshot=None,
    )
    entries = json.loads(snapshot_path.read_text(encoding="utf-8"))["entries"]
    assert set(entries) == {"/channels.json", "/assets/stable/manifest.json"}


def test_release_snapshot_fetches_every_public_deploy_file(tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "index.html").write_bytes(b"candidate index")
    (dist / "_headers").write_bytes(b"/*\n  Cache-Control: no-cache\n")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://preview.release.pages.dev"
    served = {
        f"{site}/channels.json": (dist / "channels.json").read_bytes(),
        f"{site}/index.html": (dist / "index.html").read_bytes(),
        f"{site}/assets/stable/manifest.json": manifest.read_bytes(),
    }
    cache: dict[str, Any] = {}

    def fetch(url: str) -> Any:
        result = SimpleNamespace(data=served[url], error=None)
        cache[url] = result
        return result

    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    snapshot = SNAPSHOT.release_fetch_snapshot(checker, site, dist=dist)

    assert "/index.html" in snapshot["entries"]
    assert "/_headers" not in snapshot["entries"]

    served[f"{site}/index.html"] = b"wrong deployment"
    with pytest.raises(RuntimeError, match=r"served bytes differ.*index\.html"):
        SNAPSHOT.release_fetch_snapshot(checker, site, dist=dist)


def test_strict_snapshot_retries_a_lagging_deploy_file(tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "404.html").write_bytes(b"candidate 404")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://release.capsem.org"
    attempts = 0
    cache: dict[str, Any] = {}

    def populate() -> int:
        nonlocal attempts
        attempts += 1
        return 0

    def fetch(url: str) -> Any:
        relative = url.removeprefix(f"{site}/")
        body = (dist / relative).read_bytes()
        if attempts == 1 and relative == "404.html":
            body = b"stale 404"
        result = SimpleNamespace(data=body, error=None)
        cache[url] = result
        return result

    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=populate,
        attempts=2,
        delay_seconds=0,
        snapshot_out=tmp_path / "production.json",
        expect_snapshot=None,
        require_valid=True,
        same_origin_only=False,
        dist=dist,
    )

    assert attempts == 2


def test_preview_checks_all_dist_bytes_but_snapshots_only_contract_evidence(
    tmp_path: Path,
) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "404.html").write_bytes(b"candidate 404")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://preview.release.pages.dev"
    cache: dict[str, Any] = {}

    def fetch(url: str) -> Any:
        relative = url.removeprefix(f"{site}/")
        result = SimpleNamespace(data=(dist / relative).read_bytes(), error=None)
        cache[url] = result
        return result

    def populate() -> int:
        fetch(f"{site}/channels.json")
        fetch(f"{site}/assets/stable/manifest.json")
        return 0

    snapshot_path = tmp_path / "candidate.json"
    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=populate,
        attempts=1,
        delay_seconds=0,
        snapshot_out=snapshot_path,
        expect_snapshot=None,
        require_valid=True,
        same_origin_only=False,
        dist=dist,
        include_dist_in_snapshot=False,
    )

    entries = json.loads(snapshot_path.read_text(encoding="utf-8"))["entries"]
    assert set(entries) == {"/channels.json", "/assets/stable/manifest.json"}


def test_contract_only_snapshot_still_rejects_wrong_preview_dist_bytes(tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "404.html").write_bytes(b"candidate 404")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://preview.release.pages.dev"
    cache: dict[str, Any] = {}

    def fetch(url: str) -> Any:
        relative = url.removeprefix(f"{site}/")
        body = b"wrong 404" if relative == "404.html" else (dist / relative).read_bytes()
        result = SimpleNamespace(data=body, error=None)
        cache[url] = result
        return result

    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    with pytest.raises(RuntimeError, match=r"served bytes differ.*404\.html"):
        SNAPSHOT.snapshot_distribution_bytes(
            checker,
            site,
            populate=lambda: 0,
            attempts=1,
            delay_seconds=0,
            snapshot_out=tmp_path / "candidate.json",
            expect_snapshot=None,
            require_valid=True,
            same_origin_only=False,
            dist=dist,
            include_dist_in_snapshot=False,
        )


def test_contract_only_snapshot_does_not_absorb_dist_files_after_retry(tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "404.html").write_bytes(b"candidate 404")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://preview.release.pages.dev"
    cache: dict[str, Any] = {}
    attempts = 0

    def fetch(url: str) -> Any:
        relative = url.removeprefix(f"{site}/")
        body = (dist / relative).read_bytes()
        if attempts == 1 and relative == "404.html":
            body = b"lagging 404"
        result = SimpleNamespace(data=body, error=None)
        cache[url] = result
        return result

    def populate() -> int:
        nonlocal attempts
        attempts += 1
        fetch(f"{site}/channels.json")
        fetch(f"{site}/assets/stable/manifest.json")
        return 0

    snapshot_path = tmp_path / "candidate.json"
    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=populate,
        attempts=2,
        delay_seconds=0,
        snapshot_out=snapshot_path,
        expect_snapshot=None,
        require_valid=True,
        same_origin_only=False,
        dist=dist,
        include_dist_in_snapshot=False,
    )

    entries = json.loads(snapshot_path.read_text(encoding="utf-8"))["entries"]
    assert attempts == 2
    assert set(entries) == {"/channels.json", "/assets/stable/manifest.json"}


def test_snapshot_retry_reuses_successful_external_graph_bytes(tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    manifest = dist / "assets" / "stable" / "manifest.json"
    manifest.parent.mkdir(parents=True)
    (dist / "channels.json").write_bytes(b'{"channels":{}}\n')
    (dist / "404.html").write_bytes(b"candidate 404")
    manifest.write_bytes(b'{"version":"1"}\n')
    site = "https://release.capsem.org"
    external = "https://github.example.test/release/immutable-rootfs.erofs"
    attempts = 0
    calls: dict[str, int] = {}
    cache: dict[str, Any] = {}

    def fetch(url: str) -> Any:
        if url in cache:
            return cache[url]
        calls[url] = calls.get(url, 0) + 1
        if url == external:
            body = b"immutable graph bytes"
        else:
            relative = url.removeprefix(f"{site}/")
            body = (dist / relative).read_bytes()
            if attempts == 1 and relative == "404.html":
                body = b"stale 404"
        result = SimpleNamespace(data=body, error=None)
        cache[url] = result
        return result

    def populate() -> int:
        nonlocal attempts
        attempts += 1
        fetch(f"{site}/channels.json")
        fetch(f"{site}/assets/stable/manifest.json")
        fetch(external)
        return 0

    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=populate,
        attempts=2,
        delay_seconds=0,
        snapshot_out=tmp_path / "production.json",
        expect_snapshot=None,
        require_valid=True,
        same_origin_only=False,
        dist=dist,
    )

    assert attempts == 2
    assert calls[external] == 1
    assert calls[f"{site}/channels.json"] == 2


def test_snapshot_retry_does_not_cache_external_fetch_failures(tmp_path: Path) -> None:
    site = "https://release.capsem.org"
    external = "https://github.example.test/release/immutable-rootfs.erofs"
    attempts = 0
    external_calls = 0
    cache: dict[str, Any] = {}

    def fetch(url: str) -> Any:
        nonlocal external_calls
        if url in cache:
            return cache[url]
        if url == external:
            external_calls += 1
            result = SimpleNamespace(
                data=b"immutable graph bytes" if external_calls == 2 else b"",
                error=None if external_calls == 2 else "temporary failure",
            )
        else:
            result = SimpleNamespace(data=b"{}\n", error=None)
        cache[url] = result
        return result

    def populate() -> int:
        nonlocal attempts
        attempts += 1
        fetch(f"{site}/channels.json")
        fetch(f"{site}/assets/stable/manifest.json")
        return int(fetch(external).error is not None)

    checker = SimpleNamespace(_FETCH_BYTES_CACHE=cache, fetch_bytes=fetch)
    SNAPSHOT.snapshot_distribution_bytes(
        checker,
        site,
        populate=populate,
        attempts=2,
        delay_seconds=0,
        snapshot_out=tmp_path / "production.json",
        expect_snapshot=None,
        require_valid=True,
        same_origin_only=False,
    )

    assert attempts == 2
    assert external_calls == 2


def _cloudflare_project(
    deployment_id: str,
    *,
    status: str = "success",
    environment: str = "production",
) -> dict[str, object]:
    return {
        "success": True,
        "result": {
            "name": "release",
            "canonical_deployment": {
                "id": deployment_id,
                "url": f"https://{deployment_id}.release.pages.dev",
                "environment": environment,
                "latest_stage": {"status": status},
            },
        },
    }


def test_cloudflare_capture_records_exact_successful_canonical_production() -> None:
    calls: list[tuple[str, str]] = []

    def request(method: str, path: str) -> dict[str, object]:
        calls.append((method, path))
        return _cloudflare_project("prior-id")

    assert ROLLBACK.capture_production(request, "release") == {
        "schema": ROLLBACK.STATE_SCHEMA,
        "project": "release",
        "deployment_id": "prior-id",
        "deployment_url": "https://prior-id.release.pages.dev",
    }
    assert calls == [("GET", "/pages/projects/release")]


@pytest.mark.parametrize(
    ("payload", "message"),
    [
        (_cloudflare_project("prior", status="failure"), "not a successful build"),
        (_cloudflare_project("prior", environment="preview"), "not a production"),
        (
            {"success": True, "result": {"name": "release"}},
            "has no production deployment",
        ),
    ],
)
def test_cloudflare_capture_refuses_unrestorable_state(
    payload: dict[str, object], message: str
) -> None:
    with pytest.raises(ROLLBACK.RollbackError, match=message):
        ROLLBACK.capture_production(lambda _method, _path: payload, "release")


def test_cloudflare_restore_targets_prior_id_and_waits_for_canonical_proof() -> None:
    calls: list[tuple[str, str]] = []
    project_reads = iter([_cloudflare_project("candidate-id"), _cloudflare_project("prior-id")])

    def request(method: str, path: str) -> dict[str, object]:
        calls.append((method, path))
        if method == "POST":
            return {"success": True, "result": {"id": "prior-id"}}
        return next(project_reads)

    restored = ROLLBACK.restore_production(
        request,
        {
            "schema": ROLLBACK.STATE_SCHEMA,
            "project": "release",
            "deployment_id": "prior-id",
        },
        attempts=2,
        delay_seconds=0,
        sleep=lambda _delay: None,
    )

    assert restored["deployment_id"] == "prior-id"
    assert calls == [
        ("POST", "/pages/projects/release/deployments/prior-id/rollback"),
        ("GET", "/pages/projects/release"),
        ("GET", "/pages/projects/release"),
    ]


def test_cloudflare_restore_retries_a_transient_rollback_request() -> None:
    posts = 0

    def request(method: str, _path: str) -> dict[str, object]:
        nonlocal posts
        if method == "POST":
            posts += 1
            if posts == 1:
                raise ROLLBACK.RollbackError("temporary Cloudflare failure")
            return {"success": True, "result": {"id": "prior-id"}}
        return _cloudflare_project("candidate-id" if posts == 1 else "prior-id")

    restored = ROLLBACK.restore_production(
        request,
        {
            "schema": ROLLBACK.STATE_SCHEMA,
            "project": "release",
            "deployment_id": "prior-id",
        },
        attempts=2,
        delay_seconds=0,
        sleep=lambda _delay: None,
    )

    assert restored["deployment_id"] == "prior-id"
    assert posts == 2


def test_cloudflare_restore_fails_when_canonical_never_returns_to_prior() -> None:
    def request(method: str, _path: str) -> dict[str, object]:
        if method == "POST":
            return {"success": True, "result": {"id": "prior-id"}}
        return _cloudflare_project("candidate-id")

    with pytest.raises(ROLLBACK.RollbackError, match="remained 'candidate-id'"):
        ROLLBACK.restore_production(
            request,
            {
                "schema": ROLLBACK.STATE_SCHEMA,
                "project": "release",
                "deployment_id": "prior-id",
            },
            attempts=2,
            delay_seconds=0,
            sleep=lambda _delay: None,
        )


def test_cloudflare_activation_identity_refuses_a_different_canonical_deployment() -> None:
    with pytest.raises(ROLLBACK.RollbackError, match="expected canonical 'candidate-id'"):
        ROLLBACK.wait_for_canonical(
            lambda _method, _path: _cloudflare_project("other-id"),
            "release",
            "candidate-id",
            attempts=1,
            delay_seconds=0,
            sleep=lambda _delay: None,
        )


def test_complete_dist_preserves_untouched_channel_manifest_version() -> None:
    stable = {
        "channel": "stable",
        "version": "1.0.7",
        "profiles": {},
        "packages": [],
    }
    nightly = {
        "channel": "nightly",
        "version": "1.0.8",
        "profiles": {},
        "packages": [],
    }

    assert (
        BUILD_COMPLETE.manifest_version_for_channel(
            channel="stable",
            primary_channel="stable",
            document=stable,
            primary_version="1.0.9",
        )
        == "1.0.9"
    )
    assert (
        BUILD_COMPLETE.manifest_version_for_channel(
            channel="nightly",
            primary_channel="stable",
            document=nightly,
            primary_version="1.0.9",
        )
        == "1.0.8"
    )

    with pytest.raises(ValueError, match="untouched nightly"):
        BUILD_COMPLETE.manifest_version_for_channel(
            channel="nightly",
            primary_channel="stable",
            document={"channel": "nightly", "profiles": {}, "packages": []},
            primary_version="1.0.9",
        )


def test_deploy_rejects_stale_untouched_channel(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dist = tmp_path / "dist"
    stable = dist / "assets" / "stable" / "manifest.json"
    stable.parent.mkdir(parents=True)
    stable.write_bytes(b'{"channel":"stable","version":"1.0.8"}\n')
    live_stable = stable.read_bytes()
    monkeypatch.setattr(
        DEPLOY_FRESHNESS,
        "read_live_manifest",
        lambda _release_site, channel: (
            live_stable if channel == "stable" else (_ for _ in ()).throw(AssertionError(channel))
        ),
    )

    DEPLOY_FRESHNESS.verify_untouched_channels(
        selected_channel="nightly",
        dist=dist,
        release_site="https://release.example.test",
    )

    stable.write_bytes(b'{"channel":"stable","version":"stale"}\n')
    with pytest.raises(ValueError, match="refusing to replace another channel"):
        DEPLOY_FRESHNESS.verify_untouched_channels(
            selected_channel="nightly",
            dist=dist,
            release_site="https://release.example.test",
        )


def test_live_manifest_only_treats_http_404_as_unpublished(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fail_with(code: int) -> Any:
        def missing(request: Any, *, timeout: int) -> Any:
            assert timeout == 60
            raise DEPLOY_FRESHNESS.HTTPError(request.full_url, code, "failure", {}, None)

        return missing

    monkeypatch.setattr(DEPLOY_FRESHNESS, "urlopen", fail_with(404))
    assert DEPLOY_FRESHNESS.read_live_manifest("https://release.example.test", "nightly") is None

    monkeypatch.setattr(DEPLOY_FRESHNESS, "urlopen", fail_with(503))
    with pytest.raises(DEPLOY_FRESHNESS.HTTPError, match="HTTP Error 503") as caught:
        DEPLOY_FRESHNESS.read_live_manifest("https://release.example.test", "nightly")
    caught.value.close()


def test_nightly_cannot_drop_the_live_stable_channel(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Nightly must carry the latest good stable graph for switching back."""
    dist = tmp_path / "dist"
    (dist / "assets" / "nightly").mkdir(parents=True)

    monkeypatch.setattr(
        DEPLOY_FRESHNESS,
        "read_live_manifest",
        lambda _release_site, _channel: b'{"channel":"stable","version":"1.0.8"}\n',
    )

    with pytest.raises(ValueError, match="published stable"):
        DEPLOY_FRESHNESS.verify_untouched_channels(
            selected_channel="nightly",
            dist=dist,
            release_site="https://release.example.test",
        )


def _run(
    command: list[str],
    *,
    timeout: int = 180,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
        env={**os.environ, **env} if env else None,
    )
    assert result.returncode == 0, (
        f"command failed: {' '.join(command)}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    return result


def test_untouched_public_graph_render_is_byte_idempotent(tmp_path: Path) -> None:
    fixture = json.loads(
        (
            PROJECT_ROOT
            / "tests"
            / "capsem-release"
            / "fixtures"
            / "release-graph-stable-nightly.json"
        ).read_text(encoding="utf-8")
    )
    source = tmp_path / "nightly-source.json"
    source.write_text(
        json.dumps(
            fixture["manifests"]["nightly"]["1.0.2"],
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    first = tmp_path / "first"
    second = tmp_path / "second"

    for manifest, output in (
        (source, first),
        (first / "assets" / "nightly" / "manifest.json", second),
    ):
        _run(
            [
                "cargo",
                "run",
                "-p",
                "capsem-admin",
                "--quiet",
                "--",
                "assets",
                "channel",
                "build",
                "--manifest",
                manifest.resolve().as_uri(),
                "--channel",
                "nightly",
                "--manifest-version",
                "1.0.2",
                "--generated-at",
                "2026-07-26T00:00:00Z",
                "--out-dir",
                str(output),
            ]
        )

    assert (first / "assets" / "nightly" / "manifest.json").read_bytes() == (
        second / "assets" / "nightly" / "manifest.json"
    ).read_bytes()


def _prepare_install_test_assets(path: Path) -> dict[str, Any]:
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={
            "CAPSEM_ARCH": "arm64",
            "CAPSEM_ASSETS_DIR": str(path),
        },
    )
    return json.loads((path / "arm64" / "obom.cdx.json").read_text())


def _assert_rootfs_scoped_install_test_obom(document: dict[str, Any]) -> None:
    properties = document["metadata"]["component"].get("properties", [])
    assert {
        "name": "capsem:evidence:scope",
        "value": "exported-rootfs",
    } in properties
    assert any(
        isinstance(component.get("purl"), str) and component["purl"].startswith("pkg:deb/debian/")
        for component in document["components"]
    )
    assert "cdx:osquery:category" not in json.dumps(document)


def test_install_test_assets_generate_rootfs_scoped_obom(tmp_path: Path) -> None:
    _assert_rootfs_scoped_install_test_obom(_prepare_install_test_assets(tmp_path / "assets"))


def test_install_test_assets_replace_stale_host_inventory_obom(tmp_path: Path) -> None:
    assets = tmp_path / "assets"
    obom = assets / "arm64" / "obom.cdx.json"
    obom.parent.mkdir(parents=True)
    obom.write_text(
        json.dumps(
            {
                "bomFormat": "CycloneDX",
                "specVersion": "1.6",
                "metadata": {"component": {"name": "build-host", "type": "device"}},
                "components": [
                    {
                        "name": "browser-extension-from-host",
                        "type": "application",
                        "properties": [
                            {
                                "name": "cdx:osquery:category",
                                "value": "chrome_extensions",
                            }
                        ],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    document = _prepare_install_test_assets(assets)

    _assert_rootfs_scoped_install_test_obom(document)
    assert "browser-extension-from-host" not in json.dumps(document)


def _run_admin(
    *args: str,
    timeout: int = 180,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return _run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        timeout=timeout,
        env=env,
    )


def _load_release_validator() -> Any:
    module_path = PROJECT_ROOT / "scripts" / "check-release-site-contract.py"
    spec = importlib.util.spec_from_file_location("check_release_site_contract", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _build_release_channel(
    dist: Path,
    *,
    manifest_path: Path | None = None,
    assets_dir: Path | None = None,
    asset_source_base: str | None = None,
) -> None:
    if manifest_path is None:
        prepared_assets = dist.parent / "prepared-assets"
        _run(
            ["bash", "scripts/prepare-install-test-assets.sh"],
            env={"CAPSEM_ASSETS_DIR": str(prepared_assets)},
        )
        manifest_path = prepared_assets / "manifest.json"
        assets_dir = prepared_assets
    source_manifest = manifest_path
    source_document = json.loads(source_manifest.read_text(encoding="utf-8"))
    binary_version = source_document["binaries"]["current"]
    assets_dir = assets_dir or manifest_path.parent
    graph_source = dist.parent / "graph-source"
    source_command = [
        "assets",
        "channel",
        "build",
        "--manifest",
        source_manifest.resolve().as_uri(),
        "--assets-dir",
        str(assets_dir),
        "--profiles-dir",
        str(PROJECT_ROOT / "config/profiles"),
        "--channel",
        CHANNEL,
        "--out-dir",
        str(graph_source),
    ]
    if asset_source_base is not None:
        source_command.extend(["--asset-source-base", asset_source_base])
    _run_admin(*source_command)
    working_manifest = dist.parent / "manifest-with-binary-metadata.json"
    working_manifest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(graph_source / "assets" / CHANNEL / "manifest.json", working_manifest)
    _record_test_binary_metadata(
        working_manifest,
        dist.parent / "binary-artifacts",
        version=binary_version,
    )
    manifest_url = working_manifest.resolve().as_uri()
    command = [
        "assets",
        "channel",
        "build",
        "--manifest",
        manifest_url,
        "--assets-dir",
        str(assets_dir),
        "--channel",
        CHANNEL,
        "--out-dir",
        str(dist),
    ]
    if asset_source_base is not None:
        command.extend(["--asset-source-base", asset_source_base])
    _run_admin(*command)
    shutil.copytree(
        graph_source / "profiles" / "releases",
        dist / "profiles" / "releases",
        dirs_exist_ok=True,
    )
    build_release_channel_site(dist)
    _run_admin("assets", "channel", "check", "--channel", CHANNEL, "--dist", str(dist))


def _record_test_binary_metadata(manifest_path: Path, artifacts_dir: Path, *, version: str) -> None:
    app_executable = b"capsem app install-test executable\n"
    tray_executable = b"capsem tray install-test executable\n"
    artifacts_dir.mkdir(parents=True, exist_ok=True)
    pkg_path = artifacts_dir / f"Capsem-{version}.pkg"
    deb_path = artifacts_dir / f"Capsem_{version}_arm64.deb"
    sbom_path = artifacts_dir / "capsem-sbom.spdx.json"
    _write_minimal_pkg(
        pkg_path,
        version,
        {
            "Applications/Capsem.app/Contents/MacOS/capsem-app": app_executable,
            "Applications/Capsem.app/Contents/MacOS/capsem-tray": tray_executable,
        },
    )
    _write_minimal_deb(
        deb_path,
        {
            "usr/bin/capsem-app": app_executable,
            "usr/bin/capsem-tray": tray_executable,
        },
    )
    sbom_path.write_text(
        json.dumps(
            {
                "spdxVersion": "SPDX-2.3",
                "files": [
                    {
                        "SPDXID": "SPDXRef-File-capsem-app",
                        "fileName": "/usr/bin/capsem-app",
                        "checksums": [
                            {
                                "algorithm": "SHA256",
                                "checksumValue": hashlib.sha256(app_executable).hexdigest(),
                            }
                        ],
                    },
                    {
                        "SPDXID": "SPDXRef-File-capsem-tray",
                        "fileName": "/usr/bin/capsem-tray",
                        "checksums": [
                            {
                                "algorithm": "SHA256",
                                "checksumValue": hashlib.sha256(tray_executable).hexdigest(),
                            }
                        ],
                    },
                ],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    _run_admin(
        "assets",
        "channel",
        "record-binary",
        "--manifest-path",
        str(manifest_path),
        "--version",
        version,
        "--source-commit",
        "0" * 40,
        "--artifact",
        str(pkg_path),
        "--artifact",
        str(deb_path),
        "--artifact",
        str(sbom_path),
        "--date",
        "2026-07-02",
    )


def _write_minimal_pkg(path: Path, version: str, members: dict[str, bytes]) -> None:
    root = path.parent / "pkg-root"
    if root.exists():
        shutil.rmtree(root)
    for member_path, contents in members.items():
        destination = root / member_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(contents)
        destination.chmod(0o755)
    if sys.platform != "darwin":
        payload_dir = path.parent / f"{path.stem}.expanded" / "capsem.pkg" / "Payload"
        expanded_root = payload_dir.parent.parent
        if expanded_root.exists():
            shutil.rmtree(expanded_root)
        for member_path, contents in members.items():
            destination = payload_dir / member_path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(contents)
            destination.chmod(0o755)
        with tarfile.open(path, mode="w:gz") as tar:
            tar.add(expanded_root, arcname=expanded_root.name)
        shutil.rmtree(expanded_root)
        return
    _run(
        [
            "pkgbuild",
            "--root",
            str(root),
            "--identifier",
            "org.capsem.Capsem",
            "--version",
            version,
            str(path),
        ],
        timeout=180,
    )


def _write_minimal_deb(path: Path, members: dict[str, bytes]) -> None:
    architecture = path.stem.rsplit("_", maxsplit=1)[-1]
    assert architecture in {"amd64", "arm64"}
    control = (f"Package: capsem\nVersion: 1.0.0\nArchitecture: {architecture}\n").encode()
    control_tar_gz = io.BytesIO()
    with tarfile.open(fileobj=control_tar_gz, mode="w:gz") as tar:
        info = tarfile.TarInfo("control")
        info.mode = 0o644
        info.size = len(control)
        tar.addfile(info, io.BytesIO(control))
    data_tar_gz = io.BytesIO()
    with tarfile.open(fileobj=data_tar_gz, mode="w:gz") as tar:
        for member_path, contents in members.items():
            info = tarfile.TarInfo(member_path)
            info.mode = 0o755
            info.size = len(contents)
            tar.addfile(info, io.BytesIO(contents))
    deb = bytearray(b"!<arch>\n")
    _append_ar_member(deb, "debian-binary", b"2.0\n")
    _append_ar_member(deb, "control.tar.gz", control_tar_gz.getvalue())
    _append_ar_member(deb, "data.tar.gz", data_tar_gz.getvalue())
    path.write_bytes(bytes(deb))


def _append_ar_member(out: bytearray, name: str, contents: bytes) -> None:
    header = (f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{0o100644:<8}{len(contents):<10}`\n").encode(
        "ascii"
    )
    assert len(header) == 60
    out.extend(header)
    out.extend(contents)
    if len(contents) % 2:
        out.extend(b"\n")


@pytest.fixture(scope="module")
def release_channel_dist(tmp_path_factory: pytest.TempPathFactory) -> Path:
    dist = tmp_path_factory.mktemp("release-channel") / "dist"
    _build_release_channel(dist)
    return dist


def _headers_rules(dist: Path) -> list[tuple[str, dict[str, str]]]:
    rules: list[tuple[str, dict[str, str]]] = []
    current_path: str | None = None
    current_headers: dict[str, str] = {}
    for raw_line in (dist / "_headers").read_text().splitlines():
        if not raw_line.strip() or raw_line.lstrip().startswith("#"):
            continue
        if raw_line.startswith((" ", "\t")):
            name, value = raw_line.strip().split(":", maxsplit=1)
            current_headers[name.strip()] = value.strip()
            continue
        if current_path is not None:
            rules.append((current_path, current_headers))
        current_path = raw_line.strip()
        current_headers = {}
    if current_path is not None:
        rules.append((current_path, current_headers))
    assert rules, "generated release channel must include Cloudflare _headers rules"
    return rules


def _headers_for_path(path: str, rules: list[tuple[str, dict[str, str]]]) -> dict[str, str]:
    selected: dict[str, tuple[int, str]] = {}
    for pattern, rule_headers in rules:
        specificity = _header_rule_specificity(pattern, path)
        if specificity is None:
            continue
        for name, value in rule_headers.items():
            previous = selected.get(name)
            if previous is None or specificity >= previous[0]:
                selected[name] = (specificity, value)
    return {name: value for name, (_specificity, value) in selected.items()}


def _header_rule_specificity(pattern: str, path: str) -> int | None:
    if pattern.endswith("*"):
        prefix = pattern[:-1]
        if path.startswith(prefix):
            return len(prefix)
        return None
    if path == pattern:
        return len(pattern) + 1000
    return None


@contextlib.contextmanager
def _serve_release_channel(
    dist: Path,
    *,
    header_overrides: dict[str, str] | None = None,
) -> Iterator[str]:
    rules = _headers_rules(dist)
    overrides = header_overrides or {}

    class ReleaseChannelHandler(http.server.SimpleHTTPRequestHandler):
        def end_headers(self) -> None:
            request_path = urlparse(self.path).path
            headers = _headers_for_path(request_path, rules)
            headers.update(overrides)
            for name, value in headers.items():
                self.send_header(name, value)
            super().end_headers()

        def log_message(self, format: str, *args: Any) -> None:
            return

    handler = functools.partial(ReleaseChannelHandler, directory=str(dist))
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler) as server:
        server.allow_reuse_address = True
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            host, port = server.server_address
            yield f"http://{host}:{port}"
        finally:
            server.shutdown()
            thread.join(timeout=5)


def _validate_release_site(url: str, *, capsys: pytest.CaptureFixture[str]) -> int:
    validator = _load_release_validator()
    exit_code = validator.validate_release_site(
        release_site=url,
        channel=CHANNEL,
        attempts=1,
        delay_seconds=0,
    )
    captured = capsys.readouterr()
    if exit_code != 0:
        pytest.fail(
            f"release-site validator failed for {url}\n"
            f"stdout:\n{captured.out}\n"
            f"stderr:\n{captured.err}"
        )
    return exit_code


def _validator_exit_code(url: str, *, capsys: pytest.CaptureFixture[str]) -> tuple[int, str, str]:
    validator = _load_release_validator()
    exit_code = validator.validate_release_site(
        release_site=url,
        channel=CHANNEL,
        attempts=1,
        delay_seconds=0,
    )
    captured = capsys.readouterr()
    return exit_code, captured.out, captured.err


def test_generated_release_channel_passes_public_contract(
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert (release_channel_dist / "index.html").is_file()
    assert (release_channel_dist / "channels.json").is_file()
    assert (release_channel_dist / "health.json").is_file()
    assert (release_channel_dist / "_headers").is_file()
    assert (release_channel_dist / "assets" / CHANNEL / "manifest.json").is_file()
    assert (release_channel_dist / "profiles" / "releases").is_dir()
    assert not (release_channel_dist / "assets" / "releases").exists()
    assert (release_channel_dist / "profiles" / "releases").is_dir()
    channels = json.loads((release_channel_dist / "channels.json").read_text())
    selected_manifest_url = channels["channels"][CHANNEL]["manifests"][0]["url"]
    assert selected_manifest_url == f"/assets/{CHANNEL}/manifest.json"
    assert (release_channel_dist / selected_manifest_url.lstrip("/")).is_file()
    assert "profile_catalog" not in channels["channels"][CHANNEL]
    manifest = json.loads((release_channel_dist / selected_manifest_url.lstrip("/")).read_text())
    assert "assets" not in manifest
    assert "binaries" not in manifest
    assert manifest["packages"]
    assert manifest["profiles"]
    for profile in manifest["profiles"].values():
        assert "current_binary" not in profile
        assert "current_assets" not in profile

    with _serve_release_channel(release_channel_dist) as url:
        assert _validate_release_site(url, capsys=capsys) == 0


def test_fresh_install_assets_generate_release_channel_evidence(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assets_dir = tmp_path / "assets"
    dist = tmp_path / "dist"
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={"CAPSEM_ASSETS_DIR": str(assets_dir)},
    )
    _build_release_channel(
        dist,
        manifest_path=assets_dir / "manifest.json",
        assets_dir=assets_dir,
    )

    health = json.loads((dist / "health.json").read_text())
    vm_oboms = health["evidence"]["vm_oboms"]
    attestations = health["evidence"]["attestations"]
    assert vm_oboms
    assert vm_oboms[0]["url"].startswith("/profiles/releases/")
    assert vm_oboms[0]["url"].endswith("/obom.cdx.json")
    vm_attestation = next(
        item for item in attestations if item["name"] == "github_attestations_vm_assets"
    )
    assert vm_attestation["predicate_url"] == vm_oboms[0]["url"]

    with _serve_release_channel(dist) as url:
        assert _validate_release_site(url, capsys=capsys) == 0


def _add_sha256_to_current_assets(manifest_path: Path, assets_dir: Path) -> dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    current = manifest["assets"]["current"]
    for arch, entries in manifest["assets"]["releases"][current]["arches"].items():
        for logical_name, entry in entries.items():
            entry["sha256"] = hashlib.sha256(
                (assets_dir / arch / logical_name).read_bytes()
            ).hexdigest()
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def test_channel_build_never_hydrates_historical_release_from_current_asset_paths(
    tmp_path: Path,
) -> None:
    assets_dir = tmp_path / "assets"
    dist = tmp_path / "dist"
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={"CAPSEM_ASSETS_DIR": str(assets_dir)},
    )
    manifest_path = assets_dir / "manifest.json"
    manifest = _add_sha256_to_current_assets(manifest_path, assets_dir)
    current = manifest["assets"]["current"]
    historical = json.loads(json.dumps(manifest["assets"]["releases"][current]))
    for entries in historical["arches"].values():
        for entry in entries.values():
            entry["hash"] = "f" * 64
            entry["sha256"] = "e" * 64
    manifest["assets"]["releases"]["2025.0101.1"] = historical
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    _build_release_channel(
        dist,
        manifest_path=manifest_path,
        assets_dir=assets_dir,
        asset_source_base="https://example.invalid/assets-v{asset_version}",
    )


def test_remote_channel_build_uses_manifest_digests_without_reopening_vm_blobs(
    tmp_path: Path,
) -> None:
    assets_dir = tmp_path / "assets"
    dist = tmp_path / "dist"
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={"CAPSEM_ASSETS_DIR": str(assets_dir)},
    )
    manifest_path = assets_dir / "manifest.json"
    manifest = _add_sha256_to_current_assets(manifest_path, assets_dir)
    current = manifest["assets"]["current"]
    for arch, entries in manifest["assets"]["releases"][current]["arches"].items():
        rootfs_name = next(name for name in entries if name.startswith("rootfs."))
        rootfs = assets_dir / arch / rootfs_name
        rootfs.unlink()
        rootfs.mkdir()

    _build_release_channel(
        dist,
        manifest_path=manifest_path,
        assets_dir=assets_dir,
        asset_source_base="https://example.invalid/assets-v{asset_version}",
    )


def test_local_channel_copy_fails_closed_on_asset_byte_mutation(tmp_path: Path) -> None:
    assets_dir = tmp_path / "assets"
    dist = tmp_path / "dist"
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={"CAPSEM_ASSETS_DIR": str(assets_dir)},
    )
    manifest_path = assets_dir / "manifest.json"
    manifest = _add_sha256_to_current_assets(manifest_path, assets_dir)
    current = manifest["assets"]["current"]
    arch = next(iter(manifest["assets"]["releases"][current]["arches"]))
    rootfs_name = next(
        name
        for name in manifest["assets"]["releases"][current]["arches"][arch]
        if name.startswith("rootfs.")
    )
    (assets_dir / arch / rootfs_name).write_bytes(b"tampered but locally present\n")
    result = subprocess.run(
        [
            "cargo",
            "run",
            "-p",
            "capsem-admin",
            "--quiet",
            "--",
            "assets",
            "channel",
            "build",
            "--manifest",
            manifest_path.resolve().as_uri(),
            "--assets-dir",
            str(assets_dir),
            "--channel",
            CHANNEL,
            "--out-dir",
            str(dist),
        ],
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode != 0
    assert "rootfs" in result.stderr
    assert "mismatch" in result.stderr


def test_release_channel_contract_rejects_swapped_manifest(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    manifest_path = dist / "assets" / CHANNEL / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    profile = next(iter(manifest["profiles"].values()))
    image = profile["architectures"][0]
    image["images"][0]["url"] = image["images"][0]["url"].replace(
        "/profiles/releases/stable/",
        "/profiles/releases/nightly/",
    )
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1
    assert "channel manifest SHA-256 mismatch" in stderr
    assert "channel manifest BLAKE3 mismatch" in stderr


def test_release_channel_contract_ignores_stale_health_summary(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    health_path = dist / "health.json"
    health = json.loads(health_path.read_text())
    health["current"]["assets"] = "2030.0101.1"
    health["assets"]["version"] = "2030.0101.1"
    health_path.write_text(json.dumps(health, indent=2, sort_keys=True) + "\n")

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 0
    assert "health" not in stderr.lower()


def test_release_channel_contract_rejects_cache_header_drift(
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    with _serve_release_channel(
        release_channel_dist,
        header_overrides={"Cache-Control": "public, max-age=3600"},
    ) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1
    assert "Cache-Control must contain no-cache" in stderr


def test_two_generated_release_channels_have_same_machine_contract(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    second_dist = tmp_path / "second-dist"
    _build_release_channel(second_dist)

    first_manifest = _semantic_manifest_contract(
        release_channel_dist / "assets" / CHANNEL / "manifest.json"
    )
    second_manifest = _semantic_manifest_contract(
        second_dist / "assets" / CHANNEL / "manifest.json"
    )
    assert second_manifest == first_manifest
    assert (second_dist / "_headers").read_text() == (release_channel_dist / "_headers").read_text()

    for dist in (release_channel_dist, second_dist):
        with _serve_release_channel(dist) as url:
            assert _validate_release_site(url, capsys=capsys) == 0


def _semantic_manifest_contract(path: Path) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    for package in manifest["packages"]:
        package.pop("bytes", None)
        package.pop("digest", None)
    return manifest


def test_release_channel_contract_rejects_an_html_body_for_a_missing_manifest(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A missing manifest answers `200` with the site's SPA fallback.

    Every other case here is about the manifest being *wrong* -- swapped,
    stale, mutated, digest-drifted. This is the one where it is absent, and it
    is the one that actually happens: the release site serves its index page
    for any unknown path, so a check that trusts the status code reports a
    healthy channel that has never been published to.

    Observed live: `GET /assets/nightly/manifest.json` returns 200 and
    `<!DOCTYPE html>`, because nothing has ever been published there.
    """
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    manifest_path = dist / "assets" / CHANNEL / "manifest.json"
    manifest_path.write_text(
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        "<title>capsem</title></head><body>not a manifest</body></html>\n"
    )

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1, "an HTML body was accepted as a channel manifest"
    assert "manifest JSON parse failed" in stderr, stderr


def test_release_channel_contract_rejects_a_manifest_that_is_not_an_object(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Valid JSON is not a valid manifest.

    A body that parses but is a list, a string, or `null` gets past the parse
    and into every downstream lookup, where it fails as a confusing
    `AttributeError` somewhere far from the cause.
    """
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    manifest_path = dist / "assets" / CHANNEL / "manifest.json"
    manifest_path.write_text('["channel", "stable"]\n')

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1
    assert "is not an object" in stderr, stderr
