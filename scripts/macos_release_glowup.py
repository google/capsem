#!/usr/bin/env python3
"""Build and prove the exact macOS package without a Just recipe fork."""

from __future__ import annotations

import argparse
import copy
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import cast

from macos_candidate_content import (
    hardlink_or_copy as hardlink_or_copy,
)
from macos_candidate_content import (
    localize_candidate_profile_urls,
    stage_candidate_assets,
)

from capsem.gate import config as gate_config
from capsem.gate.content import ProfileContent

try:
    from release_glowup import (
        ArtifactIdentity,
        PairingIdentity,
        TransitionKind,
        assert_manifest_artifact,
        build_report,
        build_transition_evidence,
        tamper_profile_artifact_digest,
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
        tamper_profile_artifact_digest,
        validate_installed_evidence,
    )


ROOT = Path(__file__).resolve().parent.parent
GUEST_RELEASE_ROOT = "http://127.0.0.1:18765/candidate"
GUEST_ASSET_ROOT = "file:///Volumes/My%20Shared%20Files/capsem-assets"


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    print("+", shlex.join(command), flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True)


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

    run(
        [
            str(admin),
            "assets",
            "channel",
            "record-binary",
            "--manifest-path",
            str(source_manifest),
            "--version",
            version,
            "--artifact",
            str(package),
            "--artifact",
            str(canonical_sbom),
        ],
        env={**os.environ, "CAPSEM_RELEASE_URL": release_base},
    )
    dist = work_dir / "dist"
    run(
        [
            str(admin),
            "assets",
            "channel",
            "build",
            "--manifest",
            source_manifest.resolve().as_uri(),
            "--assets-dir",
            str(content.assets),
            "--profiles-dir",
            str(content.profiles(config)),
            "--channel",
            channel,
            "--manifest-version",
            "1.0.0",
            "--asset-source-base",
            f"{GUEST_ASSET_ROOT}/{{asset_version}}",
            "--out-dir",
            str(dist),
        ],
        env={**os.environ, "CAPSEM_RELEASE_URL": release_base},
    )
    manifest_path = dist / "assets" / channel / "manifest.json"
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


def prepare_tampered_manifest(manifest_path: Path, destination: Path) -> Path:
    """Stage a digest-invalid candidate without mutating the exact authority."""

    authority = manifest_path.read_bytes()
    manifest = json.loads(authority)
    if not isinstance(manifest, dict):
        raise RuntimeError("candidate release manifest must be an object")
    tampered = copy.deepcopy(manifest)
    tamper_profile_artifact_digest(tampered)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(tampered, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if manifest_path.read_bytes() != authority:
        raise RuntimeError("tamper staging mutated the exact candidate manifest")
    if destination.read_bytes() == authority:
        raise RuntimeError("tamper staging did not change the candidate manifest")
    return destination


def _require_dict(value: object, field: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise RuntimeError(f"macOS glow-up report {field} must be an object")
    return cast(dict[str, object], value)


def finalize_native_report(
    *,
    report_path: Path,
    physical_report_path: Path,
    manifest_path: Path,
    package: Path,
    version: str,
    channel: str,
) -> dict[str, object]:
    """Merge Tart transitions with physical VZ probes under one contract."""

    tart_report = _require_dict(
        json.loads(report_path.read_text(encoding="utf-8")),
        "root",
    )
    physical_report = _require_dict(
        json.loads(physical_report_path.read_text(encoding="utf-8")),
        "physical_vz",
    )
    artifact = ArtifactIdentity.from_path(
        package,
        version=version,
        platform="macos",
        architecture="arm64",
    )
    if physical_report.get("package_sha256") != artifact.sha256:
        raise RuntimeError("physical VZ proof did not use the Tart-tested package")
    if physical_report.get("guest_vm_booted") is not True:
        raise RuntimeError("physical VZ proof did not boot the package payload")
    if physical_report.get("full_doctor") is not True:
        raise RuntimeError("physical VZ proof did not pass full installed doctor")
    if physical_report.get("installed_winterfell") is not True:
        raise RuntimeError("physical VZ proof did not pass installed Winterfell")

    installed = _require_dict(tart_report.get("installed"), "installed")
    adapter_evidence = _require_dict(
        tart_report.get("adapter_evidence"),
        "adapter_evidence",
    )
    preserved_installed = _require_dict(
        adapter_evidence.get("preserved_installed"),
        "preserved_installed",
    )
    validate_installed_evidence(installed)
    validate_installed_evidence(preserved_installed)
    if preserved_installed != installed:
        raise RuntimeError(
            "tamper rejection did not preserve the exact normalized installed state"
        )
    rejection = _require_dict(
        adapter_evidence.get("tamper_rejection"),
        "tamper_rejection",
    )
    expected_rejection = {
        "schema": "capsem.installed_rejection.v1",
        "kind": "tampered_artifact",
        "result": "rejected",
        "preserved_previous": True,
        "manifest_unchanged": True,
        "manifest_metadata_unchanged": True,
        "profiles_unchanged": True,
        "package_unchanged": True,
        "service": "ok",
        "gateway": "ok",
    }
    for field, expected in expected_rejection.items():
        if rejection.get(field) != expected:
            raise RuntimeError(
                f"macOS tamper rejection {field} is "
                f"{rejection.get(field)!r}, expected {expected!r}"
            )

    manifest_bytes = manifest_path.read_bytes()
    pairing = PairingIdentity.from_manifest_bytes(
        manifest_bytes,
        artifact=artifact,
        channel=channel,
    )
    transitions = [
        build_transition_evidence(
            kind=TransitionKind.FRESH_INSTALL,
            before=None,
            after=pairing,
            result="activated",
            doctor_passed=True,
            winterfell_passed=True,
        ),
        build_transition_evidence(
            kind=TransitionKind.TAMPER_REJECTION,
            before=pairing,
            after=pairing,
            result="rejected",
            doctor_passed=True,
            winterfell_passed=True,
            preserved_previous=True,
        ),
    ]
    capabilities = _require_dict(tart_report.get("capabilities"), "capabilities")
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
    )
    tampered_manifest = prepare_tampered_manifest(
        manifest_path,
        manifest_path.parent / "tampered-manifest.json",
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
            str(tampered_manifest),
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
