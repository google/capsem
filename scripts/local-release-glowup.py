#!/usr/bin/env python3
"""Hermetic exact-byte glow-up through capsem-admin and the public installer."""

from __future__ import annotations

import argparse
import copy
import errno
import functools
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from urllib.parse import unquote, urljoin, urlparse

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.productschema import ProfileRevisionPolicy
from capsem_builder.gate.releaseauthoring import author_native_candidate
from capsem_builder.gate.sourcecommit import SourceCommit
from marketing_install_surface import validate_checked_in_marketing_install_surface
from release_pairing_baseline import exact_channel_catalog, validate_selected_profile_scope

try:
    from release_glowup import (
        ArtifactIdentity,
        PairingIdentity,
        TransitionKind,
        artifact_identity_from_manifest_package,
        assert_manifest_artifact,
        build_report,
        build_transition_evidence,
        explicit_channel_switch_args,
        requires_changed_profiles,
        tamper_profile_artifact_digest,
        validate_installed_evidence,
        validate_pairing_inputs,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        ArtifactIdentity,
        PairingIdentity,
        TransitionKind,
        artifact_identity_from_manifest_package,
        assert_manifest_artifact,
        build_report,
        build_transition_evidence,
        explicit_channel_switch_args,
        requires_changed_profiles,
        tamper_profile_artifact_digest,
        validate_installed_evidence,
        validate_pairing_inputs,
    )

try:
    from release_fixture_server import serve_release_root
    from release_transition import validate_transition_verdict
except ModuleNotFoundError:
    from scripts.release_fixture_server import serve_release_root
    from scripts.release_transition import validate_transition_verdict

try:
    from release_first_release import (
        activates_first_profiles,
        classify_pairing_inputs,
        resolve_public_before_package,
        verify_candidate_profile_publication,
    )
except ModuleNotFoundError:
    from scripts.release_first_release import (
        activates_first_profiles,
        classify_pairing_inputs,
        resolve_public_before_package,
        verify_candidate_profile_publication,
    )

try:
    from release_inputs import load_verified_release_inputs, safe_relative, verify_payload
except ModuleNotFoundError:
    from scripts.release_inputs import (
        load_verified_release_inputs,
        safe_relative,
        verify_payload,
    )

try:
    import release_installed_probe as installed_probe
except ModuleNotFoundError:
    from scripts import release_installed_probe as installed_probe


PROJECT_ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class ExactReleasePairing:
    channel: str
    baseline_channel: str
    transition: TransitionKind
    changed_profiles: tuple[str, ...]
    #: Absent for a first release: no predecessor to identify or to boot.
    before: PairingIdentity | None
    after: PairingIdentity
    before_manifest: Path
    after_manifest: Path
    before_package: Path | None
    after_package: Path
    before_profile_inputs: Path
    after_profile_inputs: Path


@dataclass(frozen=True)
class ExactReleaseTransport:
    before_manifest: Path
    after_manifest: Path
    current_manifest: Path
    channel_catalog: Path
    current_manifest_url: str
    before_manifest_url: str
    #: The polling URL's path, so the channel catalog and the check that reads
    #: it back cannot disagree about where the manifest is served.
    current_manifest_route: str
    channel_catalog_url: str
    #: Absent for a first release, whose public-before graph serves no package.
    before_package: Path | None
    after_package: Path


@dataclass(frozen=True)
class ExactInstalledGlowupEvidence:
    fresh_transition: Path
    fresh_installed: Path
    fresh_doctor: Path
    fresh_winterfell: Path
    candidate_installed: Path
    candidate_doctor: Path
    candidate_winterfell: Path
    candidate_transition: Path
    tamper_rejection: Path
    incompatible_rejection: Path
    preserved_installed: Path
    preserved_doctor: Path
    preserved_winterfell: Path
    fresh_uses_after: bool = False


@dataclass(frozen=True)
class AdversarialExactCandidates:
    tampered_manifest: Path
    incompatible_manifest: Path


def _environment_path(name: str) -> Path | None:
    return Path(value) if (value := _environment_value(name)) else None


def _environment_value(name: str) -> str | None:
    return (os.environ.get(name) or "").strip() or None


def main() -> int:
    config = gate_config.load(PROJECT_ROOT)
    qualified_source_commit = config.environment.qualified_source_commit
    parser = argparse.ArgumentParser()
    parser.add_argument("--input-deb", required=True, type=Path)
    parser.add_argument(
        "--source-commit",
        type=SourceCommit,
        default=os.environ.get(qualified_source_commit) or None,
    )
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--assets-dir", required=True, type=Path)
    parser.add_argument("--config-root", required=True, type=Path)
    parser.add_argument(
        "--install-script", default=PROJECT_ROOT / "site/public/install.sh", type=Path
    )
    parser.add_argument(
        "--work-dir", default=PROJECT_ROOT / "target/local-release-glowup", type=Path
    )
    parser.add_argument("--evidence-dir", required=True, type=Path)
    parser.add_argument("--skip-install", action="store_true")
    parser.add_argument(
        "--package-ready",
        action="store_true",
        help="Use an already repacked publishable package without rebuilding it.",
    )
    parser.add_argument(
        "--profile-revision-policy",
        required=True,
        type=ProfileRevisionPolicy,
        choices=tuple(ProfileRevisionPolicy),
    )
    parser.add_argument(
        "--release-channel",
        choices=("stable", "nightly"),
        default=_environment_value("CAPSEM_RELEASE_CHANNEL"),
    )
    parser.add_argument(
        "--release-baseline-channel",
        choices=("stable", "nightly"),
        default=_environment_value("CAPSEM_RELEASE_BASELINE_CHANNEL"),
    )
    parser.add_argument(
        "--release-transition",
        choices=(
            "auto",
            TransitionKind.FRESH_INSTALL.value,
            TransitionKind.BINARY_ONLY.value,
            TransitionKind.PROFILE_ONLY.value,
            TransitionKind.PROFILE_THEN_BINARY.value,
        ),
        default=_environment_value("CAPSEM_RELEASE_TRANSITION"),
    )
    parser.add_argument(
        "--before-manifest",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_BEFORE_MANIFEST"),
    )
    parser.add_argument(
        "--after-manifest",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_AFTER_MANIFEST"),
    )
    parser.add_argument(
        "--before-package",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_BEFORE_PACKAGE"),
    )
    parser.add_argument(
        "--before-profile-inputs",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS"),
    )
    parser.add_argument(
        "--after-profile-inputs",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_AFTER_PROFILE_INPUTS"),
    )
    parser.add_argument("--profile", default=_environment_value("CAPSEM_RELEASE_PROFILE"))
    parser.add_argument(
        "--candidate-profile-publication",
        type=Path,
        default=_environment_path("CAPSEM_RELEASE_CANDIDATE_PROFILE_PUBLICATION"),
    )
    parser.add_argument(
        "--publication-base",
        default=_environment_value("CAPSEM_RELEASE_PUBLICATION_BASE"),
    )
    args = parser.parse_args()
    validate_checked_in_marketing_install_surface(PROJECT_ROOT)
    if args.source_commit is None:
        raise SystemExit(
            "no source commit: pass --source-commit, or run under a gate that "
            f"exports {qualified_source_commit}. This authors release "
            "provenance and will not resolve one from whatever tree it is in."
        )
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    (args.evidence_dir / "started.json").write_text(
        json.dumps({"schema": "capsem.glowup.run.v1", "package": args.input_deb.name}) + "\n",
        encoding="utf-8",
    )
    exact_pairing = validate_exact_release_pairing(args)
    if args.work_dir.exists():
        shutil.rmtree(args.work_dir)
    args.work_dir.mkdir(parents=True)
    dist = args.work_dir / "dist"
    artifacts = args.work_dir / "artifacts"
    manifests = args.work_dir / "manifests"
    for path in (dist, artifacts, manifests):
        path.mkdir(parents=True)
    report_disk_capacity(args.work_dir, "local release glow-up start")

    stable_version = nightly_version = deb_version(args.input_deb)
    arch = deb_arch(args.input_deb)
    admin = args.bin_dir / "capsem-admin"
    source_commit = args.source_commit
    if not admin.is_file() or not os.access(admin, os.X_OK):
        raise SystemExit(f"local release glow-up requires executable {admin}")

    with local_release_server(dist) as base_url:
        exact_transport: ExactReleaseTransport | None = None
        if exact_pairing is not None:
            exact_transport = stage_exact_release_transport(
                exact_pairing,
                dist=dist,
                base_url=base_url,
            )
        stable_manifest_url = f"{base_url}/assets/stable/manifest.json"
        nightly_manifest_url = f"{base_url}/assets/nightly/manifest.json"
        stable_download_base = f"{base_url}/releases/download/stable"
        nightly_download_base = f"{base_url}/releases/download/nightly"
        stable_deb = (
            artifacts / "stable" / f"v{stable_version}" / f"Capsem_{stable_version}_{arch}.deb"
        )
        nightly_deb = (
            artifacts / "nightly" / f"v{nightly_version}" / f"Capsem_{nightly_version}_{arch}.deb"
        )
        stable_sbom = artifacts / "stable" / f"v{stable_version}" / "capsem-sbom.spdx.json"
        nightly_sbom = artifacts / "nightly" / f"v{nightly_version}" / "capsem-sbom.spdx.json"

        if args.package_ready:
            stage_package_ready_artifact(args.input_deb, stable_deb)
            stage_package_ready_artifact(args.input_deb, nightly_deb)
        else:
            repack_deb(
                args.input_deb,
                stable_deb,
                args.bin_dir,
                args.config_root,
                args.assets_dir,
                stable_manifest_url,
            )
            repack_deb(
                args.input_deb,
                nightly_deb,
                args.bin_dir,
                args.config_root,
                args.assets_dir,
                nightly_manifest_url,
            )
        generate_sbom(stable_sbom, stable_deb)
        generate_sbom(nightly_sbom, nightly_deb)

        copy_artifact_tree(
            stable_deb,
            dist / "releases" / "download" / "stable" / f"v{stable_version}" / stable_deb.name,
        )
        copy_artifact_tree(
            stable_sbom,
            dist / "releases" / "download" / "stable" / f"v{stable_version}" / stable_sbom.name,
        )
        copy_artifact_tree(
            nightly_deb,
            dist / "releases" / "download" / "nightly" / f"v{nightly_version}" / nightly_deb.name,
        )
        copy_artifact_tree(
            nightly_sbom,
            dist / "releases" / "download" / "nightly" / f"v{nightly_version}" / nightly_sbom.name,
        )

        stable_manifest = manifests / "stable-assets-manifest.json"
        nightly_manifest = manifests / "nightly-assets-manifest.json"
        clone_manifest_for_channel(
            args.assets_dir / "manifest.json",
            stable_manifest,
            "stable",
        )
        report_disk_capacity(args.work_dir, "before immutable VM blob staging")
        stage_manifest_artifacts(stable_manifest, args.assets_dir, dist, base_url)
        # Project nightly from the staged single-architecture stable manifest.
        clone_manifest_for_channel(stable_manifest, nightly_manifest, "nightly")
        author_candidate = functools.partial(
            author_native_candidate,
            runner=run,
            admin=admin,
            assets_dir=args.assets_dir,
            profiles_dir=args.config_root / "profiles",
            source_commit=source_commit,
            asset_source_base=f"{base_url}/assets/releases/{{asset_version}}",
            dist=dist,
            manifest_version=config.install.manifest_version,
            profile_revision_policy=args.profile_revision_policy,
        )
        stable_channel_manifest = author_candidate(
            stable_manifest,
            channel="stable",
            version=stable_version,
            artifacts=(stable_deb, stable_sbom),
            release_environment=config.environment.release_site.runtime(url=stable_download_base),
            graph_manifest=dist / "assets" / "stable" / config.install.manifest_name,
        )
        stable_channel_sha_before_nightly = file_sha256(stable_channel_manifest)
        stable_channel_packages_before_nightly = current_package_versions(stable_channel_manifest)
        author_candidate(
            nightly_manifest,
            channel="nightly",
            version=nightly_version,
            artifacts=(nightly_deb, nightly_sbom),
            release_environment=config.environment.release_site.runtime(url=nightly_download_base),
            graph_manifest=dist / "assets" / "nightly" / config.install.manifest_name,
        )
        if file_sha256(stable_channel_manifest) != stable_channel_sha_before_nightly:
            raise SystemExit("nightly channel build mutated stable manifest")
        if (
            current_package_versions(stable_channel_manifest)
            != stable_channel_packages_before_nightly
        ):
            raise SystemExit("nightly channel build mutated stable package records")

        install_script_url = f"{base_url}/install.sh"
        shutil.copy2(args.install_script, dist / "install.sh")
        corp_dir = dist / "corp"
        corp_dir.mkdir()
        corp_manifest = corp_dir / "manifest.json"
        shutil.copy2(stable_channel_manifest, corp_manifest)
        corp_manifest_url = f"{base_url}/corp/manifest.json"
        stable_artifact = check_generated_release(
            base_url,
            stable_manifest_url,
            stable_deb,
            dist,
            "stable",
            expected_version=stable_version,
            expected_architecture=arch,
        )
        nightly_artifact = check_generated_release(
            base_url,
            nightly_manifest_url,
            nightly_deb,
            dist,
            "nightly",
            expected_version=nightly_version,
            expected_architecture=arch,
        )
        if not args.skip_install:
            if exact_pairing is not None:
                if exact_transport is None:
                    raise SystemExit("exact release transport was not staged")
                exact_evidence = run_exact_installed_glowup(
                    pairing=exact_pairing,
                    transport=exact_transport,
                    install_script_url=install_script_url,
                    release_base_url=base_url,
                    evidence_dir=args.evidence_dir / "exact-transition-evidence",
                )
                installed = json.loads(
                    exact_evidence.candidate_installed.read_text(encoding="utf-8")
                )
                installed["package_receipt"] = True
                installed["binary_cohort"] = True
                exact_artifact = artifact_identity_from_manifest_package(
                    exact_pairing.after_manifest.read_bytes(),
                    exact_pairing.after_package,
                )
                transitions = exact_installed_transition_rows(
                    exact_pairing,
                    exact_transport,
                    exact_evidence,
                )
                expected_transitions = [TransitionKind.FRESH_INSTALL]
                if not exact_evidence.fresh_uses_after:
                    expected_transitions.append(exact_pairing.transition)
                expected_transitions.append(TransitionKind.TAMPER_REJECTION)
                report = build_report(
                    adapter="linux-docker-systemd",
                    artifact=exact_artifact,
                    installed=installed,
                    capabilities={
                        "native_install": True,
                        "systemd": True,
                        "service_owned_update": True,
                        "full_doctor": True,
                        "installed_winterfell": True,
                    },
                    transitions=transitions,
                    expected_transitions=expected_transitions,
                )
                report["adapter_evidence"] = {
                    "base_url": base_url,
                    "release_pairing": {
                        "kind": exact_pairing.transition.value,
                        "before": (
                            None
                            if exact_pairing.before is None
                            else exact_pairing.before.as_report()
                        ),
                        "after": exact_pairing.after.as_report(),
                    },
                    "polled_manifest_url": exact_transport.current_manifest_url,
                    "channel_catalog_url": exact_transport.channel_catalog_url,
                    "rejections": {
                        "tampered_artifact": json.loads(
                            exact_evidence.tamper_rejection.read_text(encoding="utf-8")
                        ),
                        "incompatible_profile": json.loads(
                            exact_evidence.incompatible_rejection.read_text(encoding="utf-8")
                        ),
                    },
                }
            else:
                evidence_path = args.evidence_dir / "installed-evidence.json"
                run_installed_glowup(
                    install_script_url=install_script_url,
                    release_base_url=base_url,
                    stable_manifest_url=stable_manifest_url,
                    nightly_manifest_url=nightly_manifest_url,
                    corp_manifest_url=corp_manifest_url,
                    package_version=stable_version,
                    stable_package=stable_deb,
                    nightly_package=nightly_deb,
                    package_architecture=arch,
                    packaged_identity=(
                        installed_probe.packaged_manifest_metadata(stable_deb)
                        if args.package_ready
                        else None
                    ),
                    evidence_out=evidence_path,
                )
                installed = json.loads(evidence_path.read_text(encoding="utf-8"))
                installed["package_receipt"] = True
                installed["binary_cohort"] = True
                report = build_report(
                    adapter="linux-docker-systemd",
                    artifact=nightly_artifact,
                    installed=installed,
                    capabilities={
                        "native_install": True,
                        "systemd": True,
                        "stable_nightly_round_trip": True,
                        "corporate_channel_lock": True,
                    },
                )
                report["adapter_evidence"] = {
                    "base_url": base_url,
                    "stable_manifest_url": stable_manifest_url,
                    "nightly_manifest_url": nightly_manifest_url,
                    "stable_artifact": stable_artifact.as_report(),
                }
            (args.evidence_dir / "report.json").write_text(
                json.dumps(report, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

    print(
        "local release glow-up passed: "
        f"stable={stable_version} nightly={nightly_version} dist={dist}"
    )
    return 0


def validate_exact_release_pairing(
    args: argparse.Namespace,
) -> ExactReleasePairing | None:
    """Fail closed when a release lane supplies an incomplete exact pairing."""
    # Cleared and absent values are the same; a partial exact pairing is illegal.
    core_fields = {
        "release_channel": args.release_channel or None,
        "release_baseline_channel": args.release_baseline_channel or None,
        "release_transition": args.release_transition or None,
        "before_manifest": args.before_manifest or None,
        "after_manifest": args.after_manifest or None,
        "before_profile_inputs": args.before_profile_inputs or None,
        "after_profile_inputs": args.after_profile_inputs or None,
    }
    profile_fields = {
        "profile": args.profile or None,
        "candidate_profile_publication": args.candidate_profile_publication or None,
        "publication_base": args.publication_base or None,
    }
    if not any(value is not None for value in (*core_fields.values(), *profile_fields.values())):
        return None
    missing = [name for name, value in core_fields.items() if value is None]
    if missing:
        raise SystemExit(
            "exact pairing requires release channel, baseline channel, transition, before/after "
            "manifests, and before/after profile inputs; "
            f"missing={missing}"
        )

    channel = str(args.release_channel)
    baseline_channel = str(args.release_baseline_channel)
    before_manifest = Path(args.before_manifest)
    after_manifest = Path(args.after_manifest)
    before_profile_inputs = Path(args.before_profile_inputs)
    after_profile_inputs = Path(args.after_profile_inputs)
    before_manifest_bytes = before_manifest.read_bytes()
    after_manifest_bytes = after_manifest.read_bytes()
    before_report, _, _ = load_verified_release_inputs(before_profile_inputs)
    after_report, _, _ = load_verified_release_inputs(after_profile_inputs)
    if before_report.get("kind") != "profiles" or after_report.get("kind") != "profiles":
        raise SystemExit("exact pairing inputs must contain verified profiles")
    if (before_profile_inputs / "manifest.json").read_bytes() != before_manifest_bytes:
        raise SystemExit(
            "exact pairing before profile inputs do not reproduce the public-before manifest"
        )
    if (after_profile_inputs / "manifest.json").read_bytes() != after_manifest_bytes:
        raise SystemExit(
            "exact pairing after profile inputs do not reproduce the candidate-after manifest"
        )

    before_package, before_artifact = resolve_public_before_package(
        supplied=args.before_package, before_manifest_bytes=before_manifest_bytes
    )
    after_artifact = artifact_identity_from_manifest_package(
        after_manifest_bytes,
        Path(args.input_deb),
    )
    if args.release_transition == "auto":
        transition, changed_profiles = classify_pairing_inputs(
            channel=channel,
            baseline_channel=baseline_channel,
            before_manifest_bytes=before_manifest_bytes,
            after_manifest_bytes=after_manifest_bytes,
            before_artifact=before_artifact,
            after_artifact=after_artifact,
        )
        validate_selected_profile_scope(
            transition=transition,
            selected_profile=args.profile,
            changed_profiles=changed_profiles,
        )
    else:
        transition = TransitionKind(str(args.release_transition))
        changed_profiles = (str(args.profile),) if args.profile is not None else ()

    # Asked of the validator's own rule rather than restated. See
    # `release_glowup.requires_changed_profiles`.
    changed_profile = requires_changed_profiles(transition)
    publication_fields = {
        "candidate_profile_publication": args.candidate_profile_publication,
        "publication_base": args.publication_base,
    }
    supplied_publication_fields = [
        name for name, value in publication_fields.items() if value is not None
    ]
    if transition is TransitionKind.PROFILE_ONLY or (
        transition is TransitionKind.CHANNEL_SWITCH and args.profile is not None
    ):
        missing_profile = [name for name, value in profile_fields.items() if value is None]
        if missing_profile:
            raise SystemExit(
                "exact profile pairing requires profile, candidate publication, "
                f"and publication base; missing={missing_profile}"
            )
        verify_candidate_profile_publication(
            after_manifest=after_manifest,
            profile=args.profile,
            publication_base=args.publication_base,
            release_dir=args.candidate_profile_publication,
        )
    elif transition is TransitionKind.PROFILE_THEN_BINARY:
        if not changed_profiles:
            raise SystemExit("profile_then_binary exact pairing requires staged profiles")
        if supplied_publication_fields and len(supplied_publication_fields) != 2:
            raise SystemExit(
                "candidate profile publication base and directory must be supplied together"
            )
        if len(supplied_publication_fields) == 2:
            if args.profile is None:
                raise SystemExit("a local candidate publication requires its selected profile")
            verify_candidate_profile_publication(
                after_manifest=after_manifest,
                profile=args.profile,
                publication_base=args.publication_base,
                release_dir=args.candidate_profile_publication,
            )
    elif any(value is not None for value in profile_fields.values()):
        raise SystemExit(
            f"{transition.value} exact pairing cannot supply candidate profile publication inputs"
        )

    before, after = validate_pairing_inputs(
        kind=transition,
        channel=channel,
        baseline_channel=baseline_channel,
        before_manifest_bytes=before_manifest_bytes,
        after_manifest_bytes=after_manifest_bytes,
        before_artifact=before_artifact,
        after_artifact=after_artifact,
        changed_profiles=changed_profiles if changed_profile else (),
    )
    return ExactReleasePairing(
        channel=channel,
        baseline_channel=baseline_channel,
        transition=transition,
        changed_profiles=changed_profiles,
        before=before,
        after=after,
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        before_package=before_package,
        after_package=Path(args.input_deb),
        before_profile_inputs=before_profile_inputs,
        after_profile_inputs=after_profile_inputs,
    )


def _rewrite_transport_urls(
    value: object,
    *,
    manifest_url: str,
    replacements: dict[str, str],
    reverse: dict[str, str],
    used: set[str],
) -> None:
    if isinstance(value, dict):
        fields = cast(dict[str, object], value)
        for key, child in fields.items():
            if key == "url" and isinstance(child, str):
                absolute = urljoin(manifest_url, child)
                replacement = replacements.get(absolute)
                if replacement is not None:
                    fields[key] = replacement
                    reverse[replacement] = child
                    used.add(absolute)
                    continue
            _rewrite_transport_urls(
                child,
                manifest_url=manifest_url,
                replacements=replacements,
                reverse=reverse,
                used=used,
            )
    elif isinstance(value, list):
        for child in value:
            _rewrite_transport_urls(
                child,
                manifest_url=manifest_url,
                replacements=replacements,
                reverse=reverse,
                used=used,
            )


def _restore_transport_urls(value: object, reverse: dict[str, str]) -> None:
    if isinstance(value, dict):
        fields = cast(dict[str, object], value)
        for key, child in fields.items():
            if key == "url" and isinstance(child, str) and child in reverse:
                fields[key] = reverse[child]
            else:
                _restore_transport_urls(child, reverse)
    elif isinstance(value, list):
        for child in value:
            _restore_transport_urls(child, reverse)


def _stage_exact_transport_release(
    *,
    label: str,
    manifest_path: Path,
    package_path: Path | None,
    profile_inputs: Path,
    dist: Path,
    base_url: str,
) -> tuple[Path, Path | None]:
    manifest_bytes = manifest_path.read_bytes()
    authority = json.loads(manifest_bytes)
    if not isinstance(authority, dict):
        raise SystemExit(f"exact {label} manifest must be a JSON object")
    report, verified_manifest, _ = load_verified_release_inputs(profile_inputs)
    if report.get("kind") != "profiles" or verified_manifest != authority:
        raise SystemExit(f"exact {label} profile inputs do not reproduce their authority manifest")
    manifest_url = report.get("manifest_url")
    artifacts = report.get("artifacts")
    if not isinstance(manifest_url, str) or not isinstance(artifacts, list):
        raise SystemExit(f"exact {label} profile input report is malformed")

    root = dist / "transitions" / label
    replacements: dict[str, str] = {}
    expected_profile_urls: set[str] = set()
    for index, row_value in enumerate(artifacts):
        if not isinstance(row_value, dict):
            raise SystemExit(f"exact {label} profile input row {index} is malformed")
        row = cast(dict[str, object], row_value)
        url = row.get("url")
        if not isinstance(url, str):
            raise SystemExit(f"exact {label} profile input row {index} has no URL")
        relative = safe_relative(row.get("path"))
        source = profile_inputs / relative
        target = root / "profiles" / relative
        copy_artifact_tree(source, target)
        local_url = f"{base_url}/transitions/{label}/profiles/{relative.as_posix()}"
        replacements[url] = local_url
        expected_profile_urls.add(url)

    staged_package: Path | None = None
    package_absolute_url: str | None = None
    if package_path is not None:
        artifact = artifact_identity_from_manifest_package(manifest_bytes, package_path)
        package_record = assert_manifest_artifact(authority, artifact)
        package_url = package_record.get("url")
        if not isinstance(package_url, str):
            raise SystemExit(f"exact {label} package record has no URL")
        package_absolute_url = urljoin(manifest_url, package_url)
        staged_package = root / "package" / package_path.name
        copy_artifact_tree(package_path, staged_package)
        replacements[package_absolute_url] = (
            f"{base_url}/transitions/{label}/package/{package_path.name}"
        )

    transport = copy.deepcopy(authority)
    reverse: dict[str, str] = {}
    used: set[str] = set()
    _rewrite_transport_urls(
        transport,
        manifest_url=manifest_url,
        replacements=replacements,
        reverse=reverse,
        used=used,
    )
    if not expected_profile_urls.issubset(used):
        missing = sorted(expected_profile_urls - used)
        raise SystemExit(f"exact {label} transport omitted profile URLs: {missing}")
    if package_absolute_url is not None and package_absolute_url not in used:
        raise SystemExit(f"exact {label} transport omitted its native package URL")

    restored = copy.deepcopy(transport)
    _restore_transport_urls(restored, reverse)
    if restored != authority:
        raise SystemExit(f"exact {label} transport changed release data beyond URL projection")
    transport_manifest = root / "manifest.json"
    transport_manifest.parent.mkdir(parents=True, exist_ok=True)
    transport_manifest.write_text(
        json.dumps(transport, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return transport_manifest, staged_package


def stage_exact_release_transport(
    pairing: ExactReleasePairing,
    *,
    dist: Path,
    base_url: str,
) -> ExactReleaseTransport:
    """Build URL-only local projections around exact manifests and artifact bytes."""
    before_manifest, before_package = _stage_exact_transport_release(
        label="before",
        manifest_path=pairing.before_manifest,
        package_path=pairing.before_package,
        profile_inputs=pairing.before_profile_inputs,
        dist=dist,
        base_url=base_url,
    )
    after_manifest, after_package = _stage_exact_transport_release(
        label="after",
        manifest_path=pairing.after_manifest,
        package_path=pairing.after_package,
        profile_inputs=pairing.after_profile_inputs,
        dist=dist,
        base_url=base_url,
    )
    if after_package is None:
        raise SystemExit("exact candidate-after transport must stage its native package")
    current_route = f"/transitions/assets/{pairing.channel}/manifest.json"
    current_manifest = dist / Path(current_route.lstrip("/"))
    cross_channel = pairing.baseline_channel != pairing.channel
    initial_manifest = after_manifest if cross_channel else before_manifest
    copy_artifact_tree(initial_manifest, current_manifest)
    before_route = (
        f"/transitions/assets/{pairing.baseline_channel}/manifest.json"
        if cross_channel
        else current_route
    )
    before_current = dist / Path(before_route.lstrip("/"))
    if cross_channel:
        copy_artifact_tree(before_manifest, before_current)
    channel_catalog = dist / "transitions" / "channels.json"
    channel_catalog.write_text(
        json.dumps(
            exact_channel_catalog(
                baseline_channel=pairing.baseline_channel,
                target_channel=pairing.channel,
                before_route=before_route,
                before_manifest=before_current,
                before_blake3=file_blake3(before_current),
                target_route=current_route,
                target_manifest=current_manifest,
                target_blake3=file_blake3(current_manifest),
            ),
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return ExactReleaseTransport(
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        current_manifest=current_manifest,
        channel_catalog=channel_catalog,
        current_manifest_url=f"{base_url}{current_route}",
        before_manifest_url=f"{base_url}{before_route}",
        current_manifest_route=current_route,
        channel_catalog_url=f"{base_url}/transitions/channels.json",
        before_package=before_package,
        after_package=after_package,
    )


def promote_exact_manifest(manifest: Path, current_manifest: Path) -> None:
    """Atomically expose one already-staged manifest at the installed polling URL."""

    pending = current_manifest.with_suffix(".next")
    try:
        shutil.copyfile(manifest, pending)
        os.replace(pending, current_manifest)
    finally:
        pending.unlink(missing_ok=True)


def promote_exact_transport_manifest(
    transport: ExactReleaseTransport,
    manifest: Path,
) -> None:
    """Atomically expose and catalog-select one exact manifest's current bytes."""

    promote_exact_manifest(manifest, transport.current_manifest)
    catalog = json.loads(transport.channel_catalog.read_text(encoding="utf-8"))
    channels = catalog.get("channels")
    if not isinstance(channels, dict):
        raise SystemExit("exact transition channel catalog is malformed")
    selected = [
        manifest
        for channel in channels.values()
        if isinstance(channel, dict)
        for manifest in channel.get("manifests", [])
        if isinstance(manifest, dict)
        and manifest.get("url") == transport.current_manifest_route
        and manifest.get("status") == "current"
    ]
    if len(selected) != 1:
        raise SystemExit("exact transition channel catalog must select one current manifest")
    contents = manifest.read_bytes()
    selected[0]["version"] = json.loads(contents).get("version", selected[0].get("version"))
    selected[0]["digest"] = {
        "sha256": hashlib.sha256(contents).hexdigest(),
        "blake3": file_blake3(manifest),
    }
    pending = transport.channel_catalog.with_suffix(".next")
    try:
        pending.write_text(
            json.dumps(catalog, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        os.replace(pending, transport.channel_catalog)
    finally:
        pending.unlink(missing_ok=True)


def promote_exact_candidate_transport(transport: ExactReleaseTransport) -> None:
    """Atomically expose candidate-after bytes at the installed polling URL."""

    promote_exact_transport_manifest(transport, transport.after_manifest)


def _adversarial_profile(
    manifest: dict[str, object],
    pairing: ExactReleasePairing,
) -> tuple[str, dict[str, object]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise SystemExit("exact adversarial candidate has no profiles")
    profile_map = cast(dict[str, object], profiles)
    profile_ids = pairing.changed_profiles or tuple(sorted(profiles))
    for profile_id in profile_ids:
        profile = profile_map.get(profile_id)
        if isinstance(profile_id, str) and isinstance(profile, dict):
            return profile_id, cast(dict[str, object], profile)
    raise SystemExit("exact adversarial candidate lacks its selected profile")


def _tamper_selected_profile_digest(
    manifest: dict[str, object], pairing: ExactReleasePairing, architecture: str
) -> None:
    try:
        tamper_profile_artifact_digest(
            manifest, profile_ids=pairing.changed_profiles, architecture=architecture
        )
    except RuntimeError as error:
        raise SystemExit(f"cannot stage exact adversarial profile: {error}") from error


def stage_adversarial_exact_candidates(
    pairing: ExactReleasePairing,
    transport: ExactReleaseTransport,
    *,
    output_dir: Path,
    architecture: str,
) -> AdversarialExactCandidates:
    """Derive local rejection candidates without changing authoritative release inputs."""

    authority_before = pairing.after_manifest.read_bytes()
    projected_before = transport.after_manifest.read_bytes()
    try:
        projected = json.loads(projected_before)
    except json.JSONDecodeError as error:
        raise SystemExit(f"exact projected candidate manifest is invalid: {error}") from error
    if not isinstance(projected, dict):
        raise SystemExit("exact projected candidate manifest must be an object")

    tampered = copy.deepcopy(projected)
    _tamper_selected_profile_digest(tampered, pairing, architecture)
    incompatible = copy.deepcopy(projected)
    _, incompatible_profile = _adversarial_profile(incompatible, pairing)
    incompatible_profile["min_capsem_version"] = "9999.0.0"

    output_dir.mkdir(parents=True, exist_ok=True)
    tampered_manifest = output_dir / "tampered-artifact-manifest.json"
    incompatible_manifest = output_dir / "incompatible-profile-manifest.json"
    tampered_manifest.write_text(
        json.dumps(tampered, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    incompatible_manifest.write_text(
        json.dumps(incompatible, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if pairing.after_manifest.read_bytes() != authority_before:
        raise SystemExit("adversarial staging mutated the authoritative candidate manifest")
    if transport.after_manifest.read_bytes() != projected_before:
        raise SystemExit("adversarial staging mutated the exact projected candidate manifest")
    if tampered_manifest.read_bytes() == projected_before:
        raise SystemExit("tampered artifact candidate did not change the projected manifest")
    if incompatible_manifest.read_bytes() == projected_before:
        raise SystemExit("incompatible profile candidate did not change the projected manifest")
    return AdversarialExactCandidates(
        tampered_manifest=tampered_manifest,
        incompatible_manifest=incompatible_manifest,
    )


def run(command: list[str], *, cwd: Path = PROJECT_ROOT, env: dict[str, str] | None = None) -> None:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    print("+ " + " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, env=merged, check=True)


def deb_version(path: Path) -> str:
    return subprocess.check_output(["dpkg-deb", "-f", str(path), "Version"], text=True).strip()


def deb_arch(path: Path) -> str:
    arch = subprocess.check_output(["dpkg-deb", "-f", str(path), "Architecture"], text=True).strip()
    if arch not in {"amd64", "arm64"}:
        raise SystemExit(f"unsupported local glow-up deb architecture: {arch}")
    return arch


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def file_blake3(path: Path) -> str:
    try:
        import blake3
    except ModuleNotFoundError:
        return subprocess.check_output(["b3sum", str(path)], text=True).split()[0]
    return blake3.blake3(path.read_bytes()).hexdigest()


def current_package_versions(manifest_path: Path) -> list[str]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    return sorted(
        package["version"]
        for package in manifest.get("packages", [])
        if isinstance(package, dict) and package.get("status") == "current"
    )


def report_disk_capacity(path: Path, label: str) -> None:
    usage = shutil.disk_usage(path)
    gib = 1024**3
    print(
        f"Disk capacity ({label}): {usage.free / gib:.1f} GiB free of {usage.total / gib:.1f} GiB",
        flush=True,
    )


def repack_deb(
    input_deb: Path,
    output_deb: Path,
    bin_dir: Path,
    config_root: Path,
    assets_dir: Path,
    manifest_url: str,
) -> None:
    output_deb.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "bash",
            "build_system/packaging/linux/repack-deb.sh",
            "--manifest",
            manifest_url,
            str(input_deb),
            str(bin_dir),
            str(config_root),
            str(assets_dir),
            str(output_deb),
        ]
    )


def stage_package_ready_artifact(input_deb: Path, output_deb: Path) -> None:
    output_deb.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(input_deb, output_deb)


def generate_sbom(output: Path, deb: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    run([sys.executable, "scripts/generate-host-binary-sbom.py", "--output", str(output), str(deb)])


def copy_artifact_tree(source: Path, target: Path) -> None:
    """Stage an immutable artifact without duplicating it when possible.

    The local release server only reads these files.  A same-filesystem
    hardlink therefore preserves the exact bytes while avoiding another copy
    of the multi-gigabyte VM asset cohort late in ``just test``.  macOS and
    Linux both support this path.  Filesystems that cannot hardlink across the
    source/dist boundary retain the portable copy behavior.
    """
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(source, target)
    except OSError as error:
        fallback_errors = {
            errno.EACCES,
            errno.EPERM,
            errno.EXDEV,
            errno.ENOTSUP,
            errno.EOPNOTSUPP,
        }
        if error.errno not in fallback_errors:
            raise
        shutil.copy2(source, target)


def stage_manifest_artifacts(
    manifest_path: Path,
    assets_dir: Path,
    dist: Path,
    base_url: str,
) -> None:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if isinstance(manifest.get("profiles"), dict):
        _stage_graph_manifest_artifacts(manifest_path, manifest, dist, base_url)
        return

    assets = manifest.get("assets")
    if not isinstance(assets, dict):
        raise SystemExit("local glow-up asset manifest has no assets object")
    version = assets.get("current")
    releases = assets.get("releases")
    release = releases.get(version) if isinstance(releases, dict) else None
    arches = release.get("arches") if isinstance(release, dict) else None
    if not isinstance(version, str) or not isinstance(arches, dict):
        raise SystemExit("local glow-up asset manifest has no current release arches")
    release_dir = dist / "assets" / "releases" / version
    release_dir.mkdir(parents=True, exist_ok=True)
    for arch, rows in arches.items():
        if not isinstance(arch, str) or not isinstance(rows, dict):
            raise SystemExit("local glow-up asset manifest has invalid architecture rows")
        for logical_name, descriptor in rows.items():
            source = assets_dir / arch / logical_name
            if not source.is_file() or not isinstance(descriptor, dict):
                raise SystemExit(f"local glow-up VM asset is missing: {source}")
            expected_size = descriptor.get("size")
            if source.stat().st_size != expected_size:
                raise SystemExit(f"local glow-up VM asset size mismatch: {source}")
            # The immediately following capsem-admin channel build validates
            # every source digest against this same manifest.
            copy_artifact_tree(source, release_dir / f"{arch}-{logical_name}")


def clone_manifest_for_channel(source: Path, destination: Path, channel: str) -> None:
    manifest = json.loads(source.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict):
        raise SystemExit("local glow-up manifest must be an object")
    if isinstance(manifest.get("profiles"), dict):
        manifest["channel"] = channel
        destination.write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return
    shutil.copy2(source, destination)


def _stage_graph_manifest_artifacts(
    manifest_path: Path,
    manifest: dict[str, object],
    dist: Path,
    base_url: str,
) -> None:
    profiles = cast(dict[str, object], manifest.get("profiles"))
    if not isinstance(profiles, dict) or not profiles:
        raise SystemExit("local glow-up release graph has no profiles")

    staged: list[tuple[dict[str, object], Path, Path, bytes]] = []
    for profile_id, profile in sorted(profiles.items()):
        if (
            not isinstance(profile, dict)
            or cast(dict[str, object], profile).get("status") == "revoked"
        ):
            continue
        architectures = cast(dict[str, object], profile).get("architectures")
        if not isinstance(architectures, list) or not architectures:
            raise SystemExit(f"local glow-up release profile {profile_id} has no architectures")
        staged_architectures: list[dict[str, object]] = []
        for architecture in architectures:
            if not isinstance(architecture, dict):
                raise SystemExit(
                    f"local glow-up release profile {profile_id} has malformed architecture"
                )
            arch = cast(dict[str, object], architecture).get("architecture", "unknown")
            active_rows: list[tuple[str, int, dict[str, object], str]] = []
            for section in ("config", "images", "evidence"):
                rows = cast(dict[str, object], architecture).get(section, [])
                if not isinstance(rows, list):
                    raise SystemExit(
                        f"local glow-up release profile {profile_id}/{arch} has malformed {section}"
                    )
                for index, row in enumerate(rows):
                    if (
                        not isinstance(row, dict)
                        or cast(dict[str, object], row).get("status") == "revoked"
                    ):
                        continue
                    url = cast(dict[str, object], row).get("url")
                    if not isinstance(url, str):
                        raise SystemExit(
                            f"local glow-up release profile {profile_id}/{arch} "
                            f"{section}[{index}] has no URL"
                        )
                    active_rows.append((section, index, cast(dict[str, object], row), url))
            if not active_rows:
                raise SystemExit(
                    f"local glow-up release profile {profile_id}/{arch} has no active artifacts"
                )
            local_rows = [
                row
                for row in active_rows
                if (parsed := urlparse(row[3])).scheme == "file" and not parsed.netloc
            ]
            if local_rows and len(local_rows) != len(active_rows):
                raise SystemExit(
                    f"local glow-up release profile {profile_id}/{arch} "
                    "mixes staged and unstaged artifacts"
                )
            if not local_rows:
                continue
            staged_architectures.append(cast(dict[str, object], architecture))
            for section, index, row, url in active_rows:
                parsed = urlparse(url)
                source = Path(unquote(parsed.path))
                if not source.is_file():
                    raise SystemExit(f"local glow-up graph artifact is missing: {source}")
                payload = source.read_bytes()
                label = (
                    f"profile {profile_id}/{arch} {section}[{index}] "
                    f"{row.get('name') or row.get('path') or row.get('kind') or url}"
                )
                try:
                    verify_payload(payload, row, label)
                except ValueError as error:
                    raise SystemExit(str(error)) from error
                digest = cast(dict[str, object], row["digest"])
                sha256 = cast(str, digest["sha256"]).lower()
                filename = source.name
                if not filename or filename in {".", ".."}:
                    raise SystemExit(f"local glow-up graph artifact has unsafe name: {url}")
                relative = Path("artifacts") / "sha256" / sha256 / filename
                staged.append((row, source, dist / relative, payload))
        if not staged_architectures:
            raise SystemExit(
                f"local glow-up release profile {profile_id} has no fully staged architectures"
            )
        cast(dict[str, object], profile)["architectures"] = staged_architectures

    if not staged:
        raise SystemExit("local glow-up release graph resolved no profile artifacts")

    for row, source, target, payload in staged:
        if target.exists():
            if not target.is_file() or target.read_bytes() != payload:
                raise SystemExit(f"local glow-up graph artifact collision: {target}")
        else:
            copy_artifact_tree(source, target)
        relative = target.relative_to(dist).as_posix()
        row["url"] = f"{base_url.rstrip('/')}/{relative}"

    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def check_generated_release(
    base_url: str,
    manifest_url: str,
    expected_deb: Path,
    dist: Path,
    channel: str,
    *,
    expected_version: str | None = None,
    expected_architecture: str | None = None,
) -> ArtifactIdentity:
    request = urllib.request.Request(
        manifest_url,
        headers={"User-Agent": "capsem-release-client/1"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        manifest = json.loads(response.read())
    packages = manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        raise SystemExit(f"generated {channel} release manifest has no packages")
    package = next((item for item in packages if item.get("name") == expected_deb.name), None)
    if package is None:
        raise SystemExit(f"generated {channel} release manifest missing {expected_deb.name}")
    package_url = str(package.get("url", ""))
    if not package_url.startswith(f"{base_url}/releases/download/"):
        raise SystemExit(f"generated {channel} package URL is not local: {package.get('url')}")
    package_path = dist / package_url.removeprefix(f"{base_url}/")
    if not package_path.is_file():
        raise SystemExit(f"generated {channel} package URL is not served: {package_url}")
    inferred = re.fullmatch(r"Capsem_(.+)_(amd64|arm64)\.deb", expected_deb.name)
    if inferred is None:
        raise SystemExit(f"cannot infer release identity from {expected_deb.name}")
    artifact = ArtifactIdentity.from_path(
        expected_deb if expected_deb.is_file() else package_path,
        version=expected_version or inferred.group(1),
        platform="linux",
        architecture=expected_architecture or inferred.group(2),
    )
    if expected_version is not None and expected_architecture is not None:
        package = assert_manifest_artifact(manifest, artifact)
    profile_artifacts = release_profile_artifacts(manifest)
    missing_assets: list[str] = []
    staged_assets: list[tuple[str, Path, dict[str, object]]] = []
    for record in profile_artifacts:
        url = cast(str, record["url"])
        artifact_path = local_release_artifact_path(base_url, url, dist)
        if not artifact_path.is_file():
            missing_assets.append(url)
        else:
            staged_assets.append((url, artifact_path, record))
    if missing_assets:
        raise SystemExit(
            f"generated {channel} release is missing VM asset blob(s): " + ", ".join(missing_assets)
        )
    for url, artifact_path, record in staged_assets:
        try:
            verify_payload(
                artifact_path.read_bytes(),
                record,
                f"generated {channel} profile artifact {url}",
            )
        except ValueError as error:
            raise SystemExit(str(error)) from error
    return artifact


def local_release_artifact_path(base_url: str, url: str, dist: Path) -> Path:
    base = urlparse(base_url)
    parsed = urlparse(url)
    if parsed.query or parsed.fragment:
        raise SystemExit(f"generated VM asset URL has query or fragment: {url}")
    if parsed.scheme or parsed.netloc:
        if (parsed.scheme, parsed.netloc) != (base.scheme, base.netloc):
            raise SystemExit(f"generated VM asset URL is not local: {url}")
    elif not url.startswith("/") or url.startswith("//"):
        raise SystemExit(f"generated VM asset URL is not manifest-root-relative: {url}")
    try:
        relative = safe_relative(
            unquote(parsed.path).lstrip("/"),
            "generated VM asset URL",
        )
    except ValueError as error:
        raise SystemExit(str(error)) from error
    return dist / relative


def release_asset_urls(manifest: dict[str, object]) -> list[str]:
    return [cast(str, record["url"]) for record in release_profile_artifacts(manifest)]


def release_profile_artifacts(manifest: dict[str, object]) -> list[dict[str, object]]:
    artifacts: list[dict[str, object]] = []
    image_count = 0
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict):
        raise SystemExit("generated stable release manifest has no profile graph")
    for profile in profiles.values():
        if not isinstance(profile, dict):
            continue
        profile_fields = cast(dict[str, object], profile)
        architectures = profile_fields.get("architectures")
        if not isinstance(architectures, list):
            continue
        for architecture in architectures:
            if not isinstance(architecture, dict):
                continue
            architecture_fields = cast(dict[str, object], architecture)
            for section in ("config", "images", "evidence"):
                rows = architecture_fields.get(section)
                if not isinstance(rows, list):
                    continue
                for row in rows:
                    if not isinstance(row, dict):
                        continue
                    row_fields = cast(dict[str, object], row)
                    if isinstance(row_fields.get("url"), str):
                        artifacts.append(row_fields)
                        if section == "images":
                            image_count += 1
    if image_count == 0:
        raise SystemExit("generated stable release manifest has no VM asset URLs")
    return artifacts


def record_update_audit_marker(path: Path) -> None:
    """Capture the last installed audit line before exposing new candidate bytes."""

    audit = Path.home() / ".capsem" / "logs" / "update.log"
    lines = len(audit.read_text(encoding="utf-8").splitlines()) if audit.is_file() else 0
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"{lines}\n", encoding="utf-8")


def run_exact_installed_glowup(
    *,
    pairing: ExactReleasePairing,
    transport: ExactReleaseTransport,
    install_script_url: str,
    release_base_url: str,
    evidence_dir: Path,
) -> ExactInstalledGlowupEvidence:
    try:
        return _run_exact_installed_glowup(
            pairing=pairing,
            transport=transport,
            install_script_url=install_script_url,
            release_base_url=release_base_url,
            evidence_dir=evidence_dir,
        )
    finally:
        installed_probe.clear_accelerated_automatic_update_polling(run)


def _run_exact_installed_glowup(
    *,
    pairing: ExactReleasePairing,
    transport: ExactReleaseTransport,
    install_script_url: str,
    release_base_url: str,
    evidence_dir: Path,
) -> ExactInstalledGlowupEvidence:
    before_artifact = (
        None
        if pairing.before_package is None
        else artifact_identity_from_manifest_package(
            pairing.before_manifest.read_bytes(),
            pairing.before_package,
        )
    )
    after_artifact = artifact_identity_from_manifest_package(
        pairing.after_manifest.read_bytes(),
        pairing.after_package,
    )
    for artifact in (before_artifact, after_artifact):
        if artifact is not None and artifact.platform != "linux":
            raise SystemExit("Linux installed glow-up requires exact Linux package artifacts")
    if before_artifact is not None and before_artifact.architecture != after_artifact.architecture:
        raise SystemExit("exact installed transition cannot change package architecture")

    first_activation = activates_first_profiles(
        transition=pairing.transition, before_manifest_bytes=pairing.before_manifest.read_bytes()
    )
    if first_activation:
        promote_exact_candidate_transport(transport)
        fresh_artifact, fresh_package = after_artifact, pairing.after_package
    elif before_artifact is None or pairing.before_package is None:
        raise SystemExit("an upgrade transition cannot boot an absent public-before package")
    else:
        fresh_artifact, fresh_package = before_artifact, pairing.before_package
    evidence_dir.mkdir(parents=True, exist_ok=True)
    probe_functions = installed_probe.exact_installed_probe_shell(evidence_dir)
    fresh_transition = evidence_dir / "fresh-install-transition.json"
    fresh_manifest = transport.after_manifest if first_activation else transport.before_manifest
    fresh_channel = pairing.channel if first_activation else pairing.baseline_channel
    fresh_manifest_url = (
        transport.current_manifest_url if first_activation else transport.before_manifest_url
    )
    fresh_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
sudo apt-get remove --purge -y capsem || true
rm -rf "$HOME/.capsem"
curl -fsSL {shlex.quote(install_script_url)} | \
  CAPSEM_CHANNEL={shlex.quote(fresh_channel)} \
  CAPSEM_RELEASE_BASE_URL={shlex.quote(release_base_url)} \
  CAPSEM_MANIFEST_URL={shlex.quote(fresh_manifest_url)} sh
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" \
  CAPSEM_RELEASE_CHANNELS_URL={shlex.quote(transport.channel_catalog_url)} \
  "$HOME/.capsem/bin/capsem" update --assets --channel {shlex.quote(fresh_channel)}
observe_update_transition fresh_install activated \
  {shlex.quote(fresh_manifest_url)} \
  {shlex.quote(file_sha256(fresh_manifest))} 0 \
  {shlex.quote(str(fresh_transition))}
systemctl --user set-environment \
  CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS=2 \
  CAPSEM_AUTOMATIC_UPDATE_POLL_SECS=2
systemctl --user restart capsem.service
probe_installed_transition fresh-install \
  {shlex.quote(fresh_manifest_url)} \
  {shlex.quote(fresh_channel)} \
  {shlex.quote(fresh_artifact.version)} \
  {shlex.quote(str(fresh_package))} \
  {shlex.quote(fresh_artifact.platform)} \
  {shlex.quote(fresh_artifact.architecture.value)}
"""
    run(["bash", "-lc", fresh_script])

    candidate_transition = evidence_dir / "candidate-after-transition.json"
    if not first_activation:
        candidate_marker = evidence_dir / "candidate-after-audit-line"
        record_update_audit_marker(candidate_marker)
        promote_exact_candidate_transport(transport)
        switch = explicit_channel_switch_args(pairing.transition, pairing.channel)
        switch_command = ""
        if switch:
            switch_command = (
                'CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" '
                f"CAPSEM_RELEASE_CHANNELS_URL={shlex.quote(transport.channel_catalog_url)} "
                '"$HOME/.capsem/bin/capsem" '
                + " ".join(shlex.quote(argument) for argument in switch)
            )
        after_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
{switch_command}
observe_update_transition {shlex.quote(pairing.transition.value)} activated \
  {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(file_sha256(transport.after_manifest))} \
  "$(cat {shlex.quote(str(candidate_marker))})" \
  {shlex.quote(str(candidate_transition))}
probe_installed_transition candidate-after \
  {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(pairing.channel)} \
  {shlex.quote(after_artifact.version)} \
  {shlex.quote(str(pairing.after_package))} \
  {shlex.quote(after_artifact.platform)} \
  {shlex.quote(after_artifact.architecture.value)}
"""
        run(["bash", "-lc", after_script])

    adversarial = stage_adversarial_exact_candidates(
        pairing,
        transport,
        output_dir=evidence_dir / "adversarial",
        architecture=after_artifact.architecture.value,
    )
    tamper_evidence = evidence_dir / "tampered-rejection.json"
    tamper_marker = evidence_dir / "tampered-audit-line"
    record_update_audit_marker(tamper_marker)
    promote_exact_transport_manifest(transport, adversarial.tampered_manifest)
    tamper_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
cp "$CAPSEM_HOME_DIR/assets/manifest.json" \
  "$EVIDENCE_DIR/tampered-before-manifest.json"
profile_digest_before=$(installed_profile_tree_digest)
previous_manifest_sha=$(sha256sum "$EVIDENCE_DIR/tampered-before-manifest.json" | cut -d' ' -f1)
assert_manifest_served {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(file_sha256(adversarial.tampered_manifest))} "the tampered manifest"
systemctl --user restart capsem.service
observe_update_transition tampered_artifact rejected \
  {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(file_sha256(adversarial.tampered_manifest))} \
  "$(cat {shlex.quote(str(tamper_marker))})" \
  {shlex.quote(str(tamper_evidence))} "$previous_manifest_sha"
cmp "$EVIDENCE_DIR/tampered-before-manifest.json" \
  "$CAPSEM_HOME_DIR/assets/manifest.json"
! cmp -s {shlex.quote(str(transport.current_manifest))} \
  "$CAPSEM_HOME_DIR/assets/manifest.json"
test "$(installed_profile_tree_digest)" = "$profile_digest_before"
dpkg-query -W -f='${{Version}}' capsem \
  | grep -Fx {shlex.quote(after_artifact.version)}
"""
    try:
        run(["bash", "-lc", tamper_script])
    finally:
        promote_exact_candidate_transport(transport)

    incompatible_evidence = evidence_dir / "incompatible-rejection.json"
    incompatible_marker = evidence_dir / "incompatible-audit-line"
    record_update_audit_marker(incompatible_marker)
    promote_exact_transport_manifest(transport, adversarial.incompatible_manifest)
    incompatible_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
cp "$CAPSEM_HOME_DIR/assets/manifest.json" \
  "$EVIDENCE_DIR/incompatible-before-manifest.json"
profile_digest_before=$(installed_profile_tree_digest)
previous_manifest_sha=$(sha256sum "$EVIDENCE_DIR/incompatible-before-manifest.json" | cut -d' ' -f1)
assert_manifest_served {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(file_sha256(adversarial.incompatible_manifest))} \
  "the incompatible-profile manifest"
systemctl --user restart capsem.service
observe_update_transition incompatible_profile rejected \
  {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(file_sha256(adversarial.incompatible_manifest))} \
  "$(cat {shlex.quote(str(incompatible_marker))})" \
  {shlex.quote(str(incompatible_evidence))} "$previous_manifest_sha"
cmp "$EVIDENCE_DIR/incompatible-before-manifest.json" \
  "$CAPSEM_HOME_DIR/assets/manifest.json"
! cmp -s {shlex.quote(str(transport.current_manifest))} \
  "$CAPSEM_HOME_DIR/assets/manifest.json"
test "$(installed_profile_tree_digest)" = "$profile_digest_before"
dpkg-query -W -f='${{Version}}' capsem \
  | grep -Fx {shlex.quote(after_artifact.version)}
"""
    try:
        run(["bash", "-lc", incompatible_script])
    finally:
        promote_exact_candidate_transport(transport)

    preserved_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
systemctl --user restart capsem.service
probe_installed_transition rejection-preserved \
  {shlex.quote(transport.current_manifest_url)} \
  {shlex.quote(pairing.channel)} \
  {shlex.quote(after_artifact.version)} \
  {shlex.quote(str(pairing.after_package))} \
  {shlex.quote(after_artifact.platform)} \
  {shlex.quote(after_artifact.architecture.value)}
"""
    run(["bash", "-lc", preserved_script])

    fresh_installed = evidence_dir / "fresh-install-installed.json"
    fresh_doctor = evidence_dir / "fresh-install-doctor.json"
    fresh_winterfell = evidence_dir / "fresh-install-winterfell.json"
    return ExactInstalledGlowupEvidence(
        fresh_transition=fresh_transition,
        fresh_installed=fresh_installed,
        fresh_doctor=fresh_doctor,
        fresh_winterfell=fresh_winterfell,
        candidate_installed=(
            fresh_installed if first_activation else evidence_dir / "candidate-after-installed.json"
        ),
        candidate_doctor=(
            fresh_doctor if first_activation else evidence_dir / "candidate-after-doctor.json"
        ),
        candidate_winterfell=(
            fresh_winterfell
            if first_activation
            else evidence_dir / "candidate-after-winterfell.json"
        ),
        candidate_transition=(fresh_transition if first_activation else candidate_transition),
        tamper_rejection=tamper_evidence,
        incompatible_rejection=incompatible_evidence,
        preserved_installed=evidence_dir / "rejection-preserved-installed.json",
        preserved_doctor=evidence_dir / "rejection-preserved-doctor.json",
        preserved_winterfell=evidence_dir / "rejection-preserved-winterfell.json",
        fresh_uses_after=first_activation,
    )


def _probe_report_passed(path: Path, expected_schema: str) -> bool:
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(
            f"installed transition probe report is unreadable: {path}: {error}"
        ) from error
    if not isinstance(report, dict):
        raise SystemExit(f"installed transition probe report is not an object: {path}")
    if report.get("schema") != expected_schema or report.get("passed") is not True:
        raise SystemExit(f"installed transition probe failed: {path}: {report}")
    return True


def _validate_exact_installed_state(
    path: Path,
    pairing: PairingIdentity,
) -> None:
    try:
        installed = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"installed transition evidence is unreadable: {path}: {error}") from error
    if not isinstance(installed, dict):
        raise SystemExit(f"installed transition evidence is not an object: {path}")
    installed["package_receipt"] = True
    installed["binary_cohort"] = True
    validate_installed_evidence(installed)
    if installed.get("channel") != pairing.channel:
        raise SystemExit(f"installed transition evidence has wrong channel: {path}")
    if installed.get("package_version") != pairing.package_version:
        raise SystemExit(f"installed transition evidence has wrong package version: {path}")


def _load_transition_verdict(path: Path) -> dict[str, object]:
    try:
        verdict = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"transition verdict is unreadable: {path}: {error}") from error
    if not isinstance(verdict, dict):
        raise SystemExit(f"transition verdict is not an object: {path}")
    return cast(dict[str, object], verdict)


def _validate_transition_verdict_file(
    path: Path,
    *,
    kind: str,
    result: str,
    source: str,
    candidate_manifest_sha256: str,
    previous_manifest_sha256: str | None = None,
) -> None:
    try:
        validate_transition_verdict(
            _load_transition_verdict(path),
            kind=kind,
            result=result,
            source=source,
            candidate_manifest_sha256=candidate_manifest_sha256,
            previous_manifest_sha256=previous_manifest_sha256,
        )
    except RuntimeError as error:
        raise SystemExit(f"transition verdict failed: {path}: {error}") from error


def exact_installed_transition_rows(
    pairing: ExactReleasePairing,
    transport: ExactReleaseTransport,
    evidence: ExactInstalledGlowupEvidence,
) -> list[dict[str, object]]:
    fresh_pairing = pairing.after if evidence.fresh_uses_after else pairing.before
    if fresh_pairing is None:
        raise SystemExit("an upgrade transition must install its public-before pairing first")
    _validate_exact_installed_state(evidence.fresh_installed, fresh_pairing)
    if not evidence.fresh_uses_after:
        _validate_exact_installed_state(evidence.candidate_installed, pairing.after)
    _validate_exact_installed_state(evidence.preserved_installed, pairing.after)
    fresh_transport = (
        transport.after_manifest if evidence.fresh_uses_after else transport.before_manifest
    )
    _validate_transition_verdict_file(
        evidence.fresh_transition,
        kind="fresh_install",
        result="activated",
        source=transport.current_manifest_url,
        candidate_manifest_sha256=file_sha256(fresh_transport),
    )
    if not evidence.fresh_uses_after:
        _validate_transition_verdict_file(
            evidence.candidate_transition,
            kind=pairing.transition.value,
            result="activated",
            source=transport.current_manifest_url,
            candidate_manifest_sha256=file_sha256(transport.after_manifest),
        )
    installed_after_sha256 = file_sha256(transport.after_manifest)
    for path, kind in (
        (evidence.tamper_rejection, "tampered_artifact"),
        (evidence.incompatible_rejection, "incompatible_profile"),
    ):
        verdict = _load_transition_verdict(path)
        candidate_sha256 = verdict.get("candidate_manifest_sha256")
        if not isinstance(candidate_sha256, str) or candidate_sha256 == installed_after_sha256:
            raise SystemExit(f"{kind} verdict does not identify a distinct candidate: {path}")
        _validate_transition_verdict_file(
            path,
            kind=kind,
            result="rejected",
            source=transport.current_manifest_url,
            candidate_manifest_sha256=candidate_sha256,
            previous_manifest_sha256=installed_after_sha256,
        )
    fresh_doctor = _probe_report_passed(
        evidence.fresh_doctor,
        "capsem.installed_doctor.v1",
    )
    fresh_winterfell = _probe_report_passed(
        evidence.fresh_winterfell,
        "capsem.installed_winterfell.v1",
    )
    if evidence.fresh_uses_after:
        candidate_doctor = fresh_doctor
        candidate_winterfell = fresh_winterfell
    else:
        candidate_doctor = _probe_report_passed(
            evidence.candidate_doctor,
            "capsem.installed_doctor.v1",
        )
        candidate_winterfell = _probe_report_passed(
            evidence.candidate_winterfell,
            "capsem.installed_winterfell.v1",
        )
    preserved_doctor = _probe_report_passed(
        evidence.preserved_doctor,
        "capsem.installed_doctor.v1",
    )
    preserved_winterfell = _probe_report_passed(
        evidence.preserved_winterfell,
        "capsem.installed_winterfell.v1",
    )
    transitions = [
        build_transition_evidence(
            kind=TransitionKind.FRESH_INSTALL,
            before=None,
            after=fresh_pairing,
            result="activated",
            doctor_passed=fresh_doctor,
            winterfell_passed=fresh_winterfell,
        ),
    ]
    if not evidence.fresh_uses_after:
        transitions.append(
            build_transition_evidence(
                kind=pairing.transition,
                before=pairing.before,
                after=pairing.after,
                result="activated",
                doctor_passed=candidate_doctor,
                winterfell_passed=candidate_winterfell,
                staged_profiles_sha256=(
                    pairing.after.profiles_sha256
                    if pairing.transition is TransitionKind.PROFILE_THEN_BINARY
                    else None
                ),
            )
        )
    transitions.append(
        build_transition_evidence(
            kind=TransitionKind.TAMPER_REJECTION,
            before=pairing.after,
            after=pairing.after,
            result="rejected",
            doctor_passed=preserved_doctor,
            winterfell_passed=preserved_winterfell,
            preserved_previous=True,
        )
    )
    return transitions


def run_installed_glowup(
    *,
    install_script_url: str,
    release_base_url: str,
    stable_manifest_url: str,
    nightly_manifest_url: str,
    corp_manifest_url: str,
    package_version: str,
    stable_package: Path,
    nightly_package: Path,
    package_architecture: str,
    packaged_identity: dict[str, str] | None = None,
    evidence_out: Path | None = None,
) -> None:
    # Exact selected bytes and package-owned future polling provenance are
    # separate authorities during hermetic native installation.
    fresh_manifest_url = (
        packaged_identity["manifest_url"] if packaged_identity else stable_manifest_url
    )
    fresh_stable_channel = packaged_identity["channel"] if packaged_identity else "stable"
    fresh_nightly_manifest_url = (
        packaged_identity["manifest_url"] if packaged_identity else nightly_manifest_url
    )
    fresh_nightly_channel = packaged_identity["channel"] if packaged_identity else "nightly"

    installed_evidence = (
        evidence_out or PROJECT_ROOT / "target" / "local-release-glowup-evidence.json"
    )
    evidence_arg = shlex.quote(str(installed_evidence))
    transition_evidence_dir = installed_evidence.parent / "channel-transition-evidence"
    probe_functions = installed_probe.exact_installed_probe_shell(transition_evidence_dir)
    script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
{probe_functions}
check_update_log() {{
  event="$1"
  source="$2"
  {shlex.quote(sys.executable)} - "$event" "$source" "$HOME/.capsem/logs/update.log" <<'PY'
import json
import pathlib
import sys
event, source, path = sys.argv[1:]
rows = [json.loads(line) for line in pathlib.Path(path).read_text().splitlines()]
if not any(row.get("event") == event and row.get("source") == source for row in rows):
    raise SystemExit(f"missing correlated update audit event={{event}} source={{source}}")
PY
}}
check_origin_channel() {{
  channel="$1"
  source="$2"
  locked="$3"
  {shlex.quote(sys.executable)} - "$channel" "$source" "$locked" \
    "$HOME/.capsem/assets/manifest-metadata.json" <<'PY'
import json
import pathlib
import sys
channel, source, locked, path = sys.argv[1:]
origin = json.loads(pathlib.Path(path).read_text())
assert origin["channel"] == channel, origin
assert origin["manifest_url"] == source, origin
assert origin["channel_locked"] is (locked == "true"), origin
PY
}}
release_channels_url={release_base_url}/channels.json
sudo apt-get remove --purge -y capsem || true
rm -rf "$HOME/.capsem"
curl -fsSL {install_script_url} | CAPSEM_CHANNEL=stable CAPSEM_RELEASE_BASE_URL={release_base_url} sh
test -x "$HOME/.capsem/bin/capsem"
test -f "$HOME/.capsem/assets/manifest.json"
grep -F {fresh_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/assets/manifest-metadata.json"
stable_manifest_sha=$(sha256sum "$HOME/.capsem/assets/manifest.json" | cut -d' ' -f1)
test -f "$HOME/.capsem/logs/install.log"
grep -F "event=manifest_source source={stable_manifest_url}" "$HOME/.capsem/logs/install.log"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/logs/install.log"
grep -Fq "event=manifest_installed" "$HOME/.capsem/logs/install.log"
if grep -Fq "event=assets_hydrated" "$HOME/.capsem/logs/install.log"; then
  echo "ERROR: package installer synchronously hydrated VM assets" >&2
  exit 1
fi
grep -F "event=service_install_invoked" "$HOME/.capsem/logs/install.log"
wait_for_profile_assets code "$EVIDENCE_DIR/code-assets-after-install.json"
wait_for_profile_assets co-work "$EVIDENCE_DIR/co-work-assets-after-install.json"
check_update_log asset_update_complete {stable_manifest_url}
probe_installed_transition fresh-stable \
  {stable_manifest_url} {fresh_stable_channel} {package_version} \
  {shlex.quote(str(stable_package))} linux {shlex.quote(package_architecture)} {fresh_manifest_url}
dpkg-query -W -f='${{Version}}' capsem | grep -Fx {package_version}
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" CAPSEM_RELEASE_CHANNELS_URL="$release_channels_url" "$HOME/.capsem/bin/capsem" update --yes --channel nightly
grep -F {nightly_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/assets/manifest-metadata.json"
check_origin_channel nightly {nightly_manifest_url} false
check_update_log asset_update_complete {nightly_manifest_url}
probe_installed_transition channel-nightly \
  {nightly_manifest_url} nightly {package_version} \
  {shlex.quote(str(nightly_package))} linux {shlex.quote(package_architecture)}
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" CAPSEM_RELEASE_CHANNELS_URL="$release_channels_url" "$HOME/.capsem/bin/capsem" update --yes --channel stable
grep -F {stable_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/assets/manifest-metadata.json"
check_origin_channel stable {stable_manifest_url} false
check_update_log asset_update_complete {stable_manifest_url}
probe_installed_transition channel-stable-return \
  {stable_manifest_url} stable {package_version} \
  {shlex.quote(str(stable_package))} linux {shlex.quote(package_architecture)}
stable_manifest_sha_after_switch=$(sha256sum "$HOME/.capsem/assets/manifest.json" | cut -d' ' -f1)
test "$stable_manifest_sha" = "$stable_manifest_sha_after_switch"
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" "$HOME/.capsem/bin/capsem" update --assets --manifest {corp_manifest_url}
grep -F {corp_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/assets/manifest-metadata.json"
check_origin_channel corp {corp_manifest_url} true
check_update_log asset_update_complete {corp_manifest_url}
probe_installed_transition corporate \
  {corp_manifest_url} corp {package_version} \
  {shlex.quote(str(stable_package))} linux {shlex.quote(package_architecture)}
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" "$HOME/.capsem/bin/capsem" update --assets
grep -F {corp_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
check_update_log asset_update_complete {corp_manifest_url}
set +e
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" CAPSEM_RELEASE_CHANNELS_URL="$release_channels_url" "$HOME/.capsem/bin/capsem" update --assets --channel nightly > "$HOME/.capsem/corp-escape.log" 2>&1
corp_escape_status=$?
set -e
test "$corp_escape_status" -ne 0
grep -F "corporate channel is locked" "$HOME/.capsem/corp-escape.log"
check_origin_channel corp {corp_manifest_url} true
dpkg-query -W -f='${{Version}}' capsem | grep -Fx {package_version}
set +e
CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" "$HOME/.capsem/bin/capsem" update --assets --manifest {nightly_manifest_url} > "$HOME/.capsem/corp-repoint.log" 2>&1
corp_repoint_status=$?
set -e
test "$corp_repoint_status" -ne 0
grep -F "corporate channel is locked to" "$HOME/.capsem/corp-repoint.log"
check_origin_channel corp {corp_manifest_url} true
sudo apt-get remove --purge -y capsem || true
rm -rf "$HOME/.capsem"
curl -fsSL {install_script_url} | CAPSEM_CHANNEL=nightly CAPSEM_RELEASE_BASE_URL={release_base_url} sh
grep -F {fresh_nightly_manifest_url} "$HOME/.capsem/assets/manifest-metadata.json"
grep -F '"package_version": "{package_version}"' "$HOME/.capsem/assets/manifest-metadata.json"
probe_installed_transition final-nightly \
  {nightly_manifest_url} {fresh_nightly_channel} {package_version} \
  {shlex.quote(str(nightly_package))} linux {shlex.quote(package_architecture)} {fresh_nightly_manifest_url}
cp "$EVIDENCE_DIR/final-nightly-installed.json" {evidence_arg}
"""
    run(["bash", "-lc", script])


local_release_server = serve_release_root


if __name__ == "__main__":
    raise SystemExit(main())
