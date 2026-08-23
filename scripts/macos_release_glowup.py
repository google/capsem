#!/usr/bin/env python3
"""Build and prove the exact macOS package without a Just recipe fork."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

from macos_candidate_content import (
    hardlink_or_copy as hardlink_or_copy,
)
from macos_candidate_content import (
    localize_candidate_profile_urls,
    stage_candidate_assets,
)

from capsem.gate import config as gate_config
from capsem.gate.content import ProfileContent
from capsem.gate.releaseauthoring import author_native_candidate
from capsem.gate.sourcecommit import SourceCommit, source_commit_for_checkout

try:
    from release_glowup import (
        ArtifactIdentity,
        PairingIdentity,
        TransitionKind,
        assert_manifest_artifact,
        build_report,
        build_transition_evidence,
        validate_installed_evidence,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        ArtifactIdentity,
        PairingIdentity,
        TransitionKind,
        assert_manifest_artifact,
        build_report,
        build_transition_evidence,
        validate_installed_evidence,
    )

try:
    from release_transition_candidates import (
        TransitionCandidates,
        require_object,
        stage_transition_candidates,
        validate_complete_verdicts,
        validate_physical_evidence,
    )
except ModuleNotFoundError:
    from scripts.release_transition_candidates import (
        TransitionCandidates,
        require_object,
        stage_transition_candidates,
        validate_complete_verdicts,
        validate_physical_evidence,
    )


ROOT = Path(__file__).resolve().parent.parent
GUEST_RELEASE_ROOT = "http://127.0.0.1:18765/candidate"
GUEST_ASSET_ROOT = "file:///Volumes/My%20Shared%20Files/capsem-assets"


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    print("+", shlex.join(command), flush=True)
    subprocess.run(command, cwd=ROOT, env=merged, check=True)


def project_version() -> str:
    manifest = (ROOT / "Cargo.toml").read_text()
    workspace = re.search(
        r"(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\Z)",
        manifest,
    )
    if workspace is None:
        raise RuntimeError("Cargo.toml is missing [workspace.package]")
    version = re.search(r'(?m)^version\s*=\s*"([^"]+)"\s*$', workspace.group(1))
    if version is None:
        raise RuntimeError("Cargo.toml [workspace.package] is missing version")
    return version.group(1)


def prepare_candidate_manifest(
    *,
    package: Path,
    sbom: Path,
    version: str,
    channel: str,
    content: ProfileContent,
    config: gate_config.GateConfig,
    source_commit: SourceCommit,
) -> tuple[Path, Path, Path]:
    """Generate the candidate graph from the exact package release pipeline."""
    work_dir = ROOT / "target" / "macos-release-glowup"
    if work_dir.exists():
        shutil.rmtree(work_dir)
    work_dir.mkdir(parents=True)
    source_manifest = work_dir / "candidate-assets-manifest.json"
    shutil.copy2(content.assets / config.install.manifest_name, source_manifest)
    canonical_sbom = work_dir / "capsem-sbom.spdx.json"
    shutil.copy2(sbom, canonical_sbom)
    asset_share = stage_candidate_assets(
        source_manifest,
        source_root=content.assets,
        destination_root=work_dir / "asset-share",
    )
    admin = ROOT / "target" / "release" / "capsem-admin"
    release_base = f"{GUEST_RELEASE_ROOT}/releases/download/{channel}"
    dist = work_dir / "dist"

    manifest_path = author_native_candidate(
        source_manifest,
        runner=run,
        admin=admin,
        assets_dir=content.assets,
        profiles_dir=content.profiles(config),
        channel=channel,
        version=version,
        source_commit=source_commit,
        artifacts=(package, canonical_sbom),
        release_environment=config.environment.release_site.runtime(url=release_base),
        asset_source_base=f"{GUEST_ASSET_ROOT}/{{asset_version}}",
        dist=dist,
        graph_manifest=dist / "assets" / channel / config.install.manifest_name,
        manifest_version=config.install.manifest_version,
    )
    localize_candidate_profile_urls(manifest_path)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    artifact = ArtifactIdentity.from_path(
        package,
        version=version,
        platform="macos",
        architecture="arm64",
    )
    assert_manifest_artifact(manifest, artifact)
    return manifest_path, asset_share, dist


def finalize_native_report(
    *,
    report_path: Path,
    physical_report_path: Path,
    manifest_path: Path,
    candidates: TransitionCandidates,
    package: Path,
    version: str,
    channel: str,
) -> dict[str, object]:
    """Merge Tart transitions with physical VZ probes under one contract."""

    tart_report = require_object(json.loads(report_path.read_text(encoding="utf-8")), "root")
    physical_report = require_object(
        json.loads(physical_report_path.read_text(encoding="utf-8")), "physical_vz"
    )
    artifact = ArtifactIdentity.from_path(
        package,
        version=version,
        platform="macos",
        architecture="arm64",
    )
    validate_physical_evidence(physical_report, artifact.sha256)

    installed = require_object(tart_report.get("installed"), "installed")
    adapter_evidence = require_object(tart_report.get("adapter_evidence"), "adapter_evidence")
    preserved_installed = require_object(
        adapter_evidence.get("preserved_installed"), "preserved_installed"
    )
    validate_installed_evidence(installed)
    validate_installed_evidence(preserved_installed)
    if preserved_installed != installed:
        raise RuntimeError("tamper rejection did not preserve the exact normalized installed state")
    fresh_verdict = require_object(adapter_evidence.get("fresh_transition"), "fresh_transition")
    rejection = require_object(adapter_evidence.get("tamper_rejection"), "tamper_rejection")
    update_verdict = require_object(adapter_evidence.get("update_transition"), "update_transition")
    incompatible_rejection = require_object(
        adapter_evidence.get("incompatible_rejection"), "incompatible_rejection"
    )

    manifest_bytes = manifest_path.read_bytes()
    original_pairing = PairingIdentity.from_manifest_bytes(
        manifest_bytes,
        artifact=artifact,
        channel=channel,
    )
    updated_bytes = candidates.updated.read_bytes()
    updated_pairing = PairingIdentity.from_manifest_bytes(
        updated_bytes,
        artifact=artifact,
        channel=channel,
    )
    if original_pairing.profiles_sha256 == updated_pairing.profiles_sha256:
        raise RuntimeError("macOS transition candidate did not change profile identity")
    manifest_source = installed.get("manifest_url")
    if not isinstance(manifest_source, str):
        raise RuntimeError("macOS installed evidence omitted its manifest source")
    validate_complete_verdicts(
        fresh_verdict,
        update_verdict,
        rejection,
        incompatible_rejection,
        source=manifest_source,
        original_sha256=original_pairing.manifest_sha256,
        updated_sha256=updated_pairing.manifest_sha256,
        tampered_sha256=hashlib.sha256(candidates.tampered.read_bytes()).hexdigest(),
        incompatible_sha256=hashlib.sha256(candidates.incompatible.read_bytes()).hexdigest(),
    )
    transitions = [
        build_transition_evidence(
            kind=TransitionKind.FRESH_INSTALL,
            before=None,
            after=original_pairing,
            result="activated",
            doctor_passed=True,
            winterfell_passed=True,
        ),
        build_transition_evidence(
            kind=TransitionKind.PROFILE_ONLY,
            before=original_pairing,
            after=updated_pairing,
            result="activated",
            doctor_passed=True,
            winterfell_passed=True,
        ),
        build_transition_evidence(
            kind=TransitionKind.TAMPER_REJECTION,
            before=updated_pairing,
            after=updated_pairing,
            result="rejected",
            doctor_passed=True,
            winterfell_passed=True,
            preserved_previous=True,
        ),
    ]
    capabilities = require_object(tart_report.get("capabilities"), "capabilities")
    capabilities["physical_vz_boot"] = True
    capabilities["full_doctor"] = True
    capabilities["installed_winterfell"] = True
    adapter_evidence["physical_vz"] = physical_report
    final_report = build_report(
        adapter="macos-tart-launchd",
        artifact=artifact,
        installed=installed,
        capabilities=capabilities,
        transitions=transitions,
        expected_transitions=(
            TransitionKind.FRESH_INSTALL,
            TransitionKind.PROFILE_ONLY,
            TransitionKind.TAMPER_REJECTION,
        ),
    )
    final_report["adapter_evidence"] = adapter_evidence
    report_path.write_text(
        json.dumps(final_report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return final_report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=project_version())
    parser.add_argument(
        "--content-root",
        required=True,
        help="one IronBank-proved root containing the paired assets and config trees",
    )
    parser.add_argument(
        "--channel",
        choices=("stable", "nightly"),
        default=os.environ.get("CAPSEM_INSTALL_CHANNEL", "stable"),
    )
    args = parser.parse_args()

    if platform.system() != "Darwin":
        raise RuntimeError("the macOS release glow-up requires macOS")
    config = gate_config.load(ROOT)
    content_root = Path(args.content_root)
    content = ProfileContent.isolated(
        config,
        content_root if content_root.is_absolute() else ROOT / content_root,
    )
    content.require_complete(config)
    manifest_url = f"{GUEST_RELEASE_ROOT}/assets/{args.channel}/manifest.json"

    frontend_env = os.environ.copy()
    frontend_env["CI"] = "true"
    run(
        ["pnpm", "--dir", "frontend", "install", "--frozen-lockfile"],
        env=frontend_env,
    )
    run(
        [
            "bash",
            "scripts/build-test-macos-package.sh",
            "--version",
            args.version,
            "--manifest-url",
            manifest_url,
            "--assets-dir",
            str(content.assets),
            "--config-root",
            str(content.config),
        ]
    )
    package = ROOT / "packages" / f"Capsem-{args.version}.pkg"
    sbom = ROOT / "target" / "macos-package-sbom.spdx.json"
    manifest_path, asset_share, profile_share = prepare_candidate_manifest(
        package=package,
        sbom=sbom,
        version=args.version,
        channel=args.channel,
        content=content,
        config=config,
        source_commit=source_commit_for_checkout(ROOT),
    )
    candidates = stage_transition_candidates(
        manifest_path,
        manifest_path.parent,
    )
    run(
        [
            sys.executable,
            "scripts/macos_tart_glowup.py",
            "--package",
            str(package),
            "--version",
            args.version,
            "--manifest-url",
            manifest_url,
            "--manifest-file",
            str(manifest_path),
            "--tampered-manifest-file",
            str(candidates.tampered),
            "--updated-manifest-file",
            str(candidates.updated),
            "--incompatible-manifest-file",
            str(candidates.incompatible),
            "--sbom",
            str(sbom),
            "--asset-share",
            str(asset_share),
            "--profile-share",
            str(profile_share),
            "--channel",
            args.channel,
        ]
    )
    run(
        [
            "bash",
            "scripts/prove-macos-package-boot.sh",
            "--package",
            str(package),
            "--version",
            args.version,
            "--assets-dir",
            str(content.assets),
        ]
    )
    tart_report_path = ROOT / "target" / "macos-tart-glowup" / "report.json"
    physical_report_path = ROOT / "target" / "macos-package-boot" / "report.json"
    finalize_native_report(
        report_path=tart_report_path,
        physical_report_path=physical_report_path,
        manifest_path=manifest_path,
        candidates=candidates,
        package=package,
        version=args.version,
        channel=args.channel,
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"macOS release glow-up failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
