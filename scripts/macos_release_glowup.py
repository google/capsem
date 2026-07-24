#!/usr/bin/env python3
"""Build and prove the exact macOS package without a Just recipe fork."""

from __future__ import annotations

import argparse
import errno
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess
import sys

try:
    from release_glowup import ArtifactIdentity, assert_manifest_artifact
except ModuleNotFoundError:
    from scripts.release_glowup import ArtifactIdentity, assert_manifest_artifact


ROOT = Path(__file__).resolve().parent.parent
GUEST_RELEASE_ROOT = (
    "file:///Volumes/My%20Shared%20Files/capsem-release/candidate"
)
GUEST_ASSET_ROOT = "file:///Volumes/My%20Shared%20Files/capsem-assets"
GUEST_PROFILE_ROOT = "file:///Volumes/My%20Shared%20Files/capsem-profiles"


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
) -> tuple[Path, Path, Path]:
    """Generate the candidate graph from the exact package release pipeline."""

    work_dir = ROOT / "target" / "macos-release-glowup"
    if work_dir.exists():
        shutil.rmtree(work_dir)
    work_dir.mkdir(parents=True)
    source_manifest = work_dir / "candidate-assets-manifest.json"
    shutil.copy2(ROOT / "assets" / "manifest.json", source_manifest)
    canonical_sbom = work_dir / "capsem-sbom.spdx.json"
    shutil.copy2(sbom, canonical_sbom)
    asset_share = stage_candidate_assets(
        source_manifest,
        source_root=ROOT / "assets",
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
            str(ROOT / "assets"),
            "--profiles-dir",
            str(ROOT / "target" / "config" / "profiles"),
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


def hardlink_or_copy(source: Path, destination: Path) -> None:
    """Stage immutable bytes cheaply, copying only across filesystems."""

    try:
        os.link(source, destination)
    except OSError as error:
        if error.errno != errno.EXDEV:
            raise
        shutil.copyfile(source, destination)


def stage_candidate_assets(
    manifest_path: Path,
    *,
    source_root: Path,
    destination_root: Path,
) -> Path:
    """Expose exact local assets to Tart without duplicating their bytes."""

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = manifest.get("assets")
    current = assets.get("current") if isinstance(assets, dict) else None
    releases = assets.get("releases") if isinstance(assets, dict) else None
    release = releases.get(current) if isinstance(releases, dict) else None
    arches = release.get("arches") if isinstance(release, dict) else None
    if not isinstance(current, str) or not isinstance(arches, dict):
        raise RuntimeError("candidate asset manifest has no current architecture cohort")
    release_dir = destination_root / current
    release_dir.mkdir(parents=True)
    for architecture, descriptors in arches.items():
        if not isinstance(architecture, str) or not isinstance(descriptors, dict):
            raise RuntimeError("candidate asset manifest has malformed architecture rows")
        for logical_name, descriptor in descriptors.items():
            source = source_root / architecture / logical_name
            if not source.is_file() or not isinstance(descriptor, dict):
                raise RuntimeError(f"candidate asset is missing: {source}")
            if source.stat().st_size != descriptor.get("size"):
                raise RuntimeError(f"candidate asset size mismatch: {source}")
            hardlink_or_copy(
                source,
                release_dir / f"{architecture}-{logical_name}",
            )
    return destination_root


def localize_candidate_profile_urls(manifest_path: Path) -> None:
    """Point generated profile config rows at the Tart profile share."""

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    rewritten = 0
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict):
        raise RuntimeError("candidate release manifest has no profiles")
    for profile in profiles.values():
        if not isinstance(profile, dict):
            continue
        architectures = profile.get("architectures")
        if not isinstance(architectures, list):
            continue
        for architecture in architectures:
            if not isinstance(architecture, dict):
                continue
            config = architecture.get("config")
            if not isinstance(config, list):
                continue
            for row in config:
                url = row.get("url") if isinstance(row, dict) else None
                if isinstance(url, str) and url.startswith("/profiles/releases/"):
                    row["url"] = f"{GUEST_PROFILE_ROOT}{url}"
                    rewritten += 1
    if rewritten == 0:
        raise RuntimeError("candidate release manifest has no profile URLs to localize")
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=project_version())
    parser.add_argument(
        "--channel",
        choices=("stable", "nightly"),
        default=os.environ.get("CAPSEM_INSTALL_CHANNEL", "stable"),
    )
    args = parser.parse_args()

    if platform.system() != "Darwin":
        raise RuntimeError("the macOS release glow-up requires macOS")
    manifest_url = f"{GUEST_RELEASE_ROOT}/assets/{args.channel}/manifest.json"

    frontend_env = os.environ.copy()
    frontend_env["CI"] = "true"
    run(
        ["pnpm", "--dir", "frontend", "install", "--frozen-lockfile"],
        env=frontend_env,
    )
    run(["bash", "scripts/materialize-config.sh"])
    run(
        [
            "bash",
            "scripts/build-test-macos-package.sh",
            "--version",
            args.version,
            "--manifest-url",
            manifest_url,
        ]
    )
    package = ROOT / "packages" / f"Capsem-{args.version}.pkg"
    sbom = ROOT / "target" / "macos-package-sbom.spdx.json"
    manifest_path, asset_share, profile_share = prepare_candidate_manifest(
        package=package,
        sbom=sbom,
        version=args.version,
        channel=args.channel,
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
    run(["bash", "scripts/prove-macos-package-boot.sh", str(package), args.version])
    tart_report_path = ROOT / "target" / "macos-tart-glowup" / "report.json"
    physical_report_path = ROOT / "target" / "macos-package-boot" / "report.json"
    tart_report = json.loads(tart_report_path.read_text(encoding="utf-8"))
    physical_report = json.loads(physical_report_path.read_text(encoding="utf-8"))
    if physical_report.get("package_sha256") != tart_report["artifact"]["sha256"]:
        raise RuntimeError("physical VZ proof did not use the Tart-tested package")
    if physical_report.get("guest_vm_booted") is not True:
        raise RuntimeError("physical VZ proof did not boot the package payload")
    tart_report["capabilities"]["physical_vz_boot"] = True
    tart_report["adapter_evidence"]["physical_vz"] = physical_report
    tart_report_path.write_text(
        json.dumps(tart_report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"macOS release glow-up failed: {error}", file=sys.stderr)
        raise SystemExit(1)
