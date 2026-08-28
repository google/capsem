from __future__ import annotations

import ast
import hashlib
import importlib.util
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import embedded_shell
import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
MODULE_PATH = PROJECT_ROOT / "scripts" / "release_glowup.py"
TRANSITION_PATH = PROJECT_ROOT / "scripts" / "release_transition.py"
LOCAL_GLOWUP_PATH = PROJECT_ROOT / "scripts" / "local-release-glowup.py"
INSTALLED_PROBE_PATH = PROJECT_ROOT / "scripts" / "release_installed_probe.py"
MARKETING_SURFACE_PATH = PROJECT_ROOT / "scripts" / "marketing_install_surface.py"
AUTOMATIC_UPDATE_POLL_CLEANUP = [
    "systemctl",
    "--user",
    "unset-environment",
    "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS",
    "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS",
]


def _load_module():
    spec = importlib.util.spec_from_file_location("release_glowup", MODULE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _load_transition_module():
    spec = importlib.util.spec_from_file_location("release_transition", TRANSITION_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _load_local_glowup():
    spec = importlib.util.spec_from_file_location(
        "local_release_glowup",
        LOCAL_GLOWUP_PATH,
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(PROJECT_ROOT / "scripts"))
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(PROJECT_ROOT / "scripts"))
    return module


def _load_marketing_surface():
    spec = importlib.util.spec_from_file_location(
        "marketing_install_surface",
        MARKETING_SURFACE_PATH,
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _load_first_release():
    """Load the first-release classifier, which imports `release_glowup` itself."""
    path = PROJECT_ROOT / "scripts" / "release_first_release.py"
    spec = importlib.util.spec_from_file_location("release_first_release", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(PROJECT_ROOT / "scripts"))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(PROJECT_ROOT / "scripts"))
    return module


def test_local_glowup_requires_the_public_install_command_to_be_discoverable() -> None:
    module = _load_marketing_surface()

    module.validate_marketing_install_surface(
        "<main><Hero /><CTA /><code>curl -fsSL https://capsem.org/install.sh | sh</code></main>",
        install_script_url="https://capsem.org/install.sh",
    )

    with pytest.raises(SystemExit, match="does not expose the supported install command"):
        module.validate_marketing_install_surface(
            "<main>Available Summer 2026</main>",
            install_script_url="https://capsem.org/install.sh",
        )


def _artifact(tmp_path: Path, module):
    package = tmp_path / "Capsem-1.5.100.pkg"
    package.write_bytes(b"exact candidate package")
    return module.ArtifactIdentity.from_path(
        package,
        version="1.5.100",
        platform="macos",
        architecture="arm64",
    )


def _manifest(artifact) -> dict[str, object]:
    return {
        "packages": [
            {
                "name": artifact.name,
                "version": artifact.version,
                "platform": artifact.platform,
                "architecture": artifact.architecture,
                "bytes": artifact.bytes,
                "digest": {"sha256": artifact.sha256},
                "status": "current",
            }
        ],
        "profiles": {"work": {}},
    }


def test_tamper_candidate_targets_the_installed_architecture() -> None:
    module = _load_module()
    manifest = {
        "profiles": {
            "code": {
                "architectures": [
                    {
                        "architecture": "arm64",
                        "images": [
                            {
                                "status": "current",
                                "digest": {
                                    "sha256": "a" * 64,
                                    "blake3": "b" * 64,
                                },
                            }
                        ],
                    },
                    {
                        "architecture": "x86_64",
                        "images": [
                            {
                                "status": "current",
                                "digest": {
                                    "sha256": "c" * 64,
                                    "blake3": "d" * 64,
                                },
                            }
                        ],
                    },
                ]
            }
        }
    }

    module.tamper_profile_artifact_digest(
        manifest,
        profile_ids=("code",),
        architecture="x86_64",
    )

    architectures = cast(list[dict[str, Any]], manifest["profiles"]["code"]["architectures"])
    assert architectures[0]["images"][0]["digest"] == {
        "sha256": "a" * 64,
        "blake3": "b" * 64,
    }
    assert architectures[1]["images"][0]["digest"] == {
        "sha256": "0" * 64,
        "blake3": "0" * 64,
    }


def test_tamper_candidate_refuses_non_consumed_evidence() -> None:
    module = _load_module()
    manifest = {
        "profiles": {
            "code": {
                "architectures": [
                    {
                        "architecture": "x86_64",
                        "evidence": [
                            {
                                "status": "current",
                                "digest": {
                                    "sha256": "a" * 64,
                                    "blake3": "b" * 64,
                                },
                            }
                        ],
                    }
                ]
            }
        }
    }

    with pytest.raises(
        module.GlowupContractError,
        match="no consumed current profile artifact",
    ):
        module.tamper_profile_artifact_digest(
            manifest,
            profile_ids=("code",),
            architecture="x86_64",
        )

    assert manifest["profiles"]["code"]["architectures"][0]["evidence"][0]["digest"] == {
        "sha256": "a" * 64,
        "blake3": "b" * 64,
    }


def _pairing(
    module,
    *,
    channel: str,
    manifest_sha256: str,
    package_version: str,
    package_sha256: str,
    profiles_sha256: str,
):
    return module.PairingIdentity(
        channel=channel,
        manifest_sha256=manifest_sha256,
        package_version=package_version,
        package_sha256=package_sha256,
        profiles_sha256=profiles_sha256,
    )


def _transition_verdict(
    *,
    kind: str,
    result: str,
    source: str,
    candidate_sha256: str,
    installed_sha256: str,
    previous_sha256: str | None = None,
) -> dict[str, object]:
    verdict: dict[str, object] = {
        "schema": "capsem.release_transition_verdict.v1",
        "kind": kind,
        "result": result,
        "source": source,
        "candidate_manifest_sha256": candidate_sha256,
        "fetched": True,
        "installed_manifest_sha256": installed_sha256,
        "preserved_previous": result == "rejected",
        "fetch_event": "release_candidate_fetched",
        "terminal_event": (
            "release_candidate_activated" if result == "activated" else "release_candidate_rejected"
        ),
    }
    if previous_sha256 is not None:
        verdict["previous_manifest_sha256"] = previous_sha256
    return verdict


def _transition(
    module,
    kind,
    *,
    before,
    after,
    result="activated",
    staged_profiles_sha256=None,
):
    return module.build_transition_evidence(
        kind=kind,
        before=before,
        after=after,
        result=result,
        doctor_passed=True,
        winterfell_passed=True,
        staged_profiles_sha256=staged_profiles_sha256,
        preserved_previous=result == "rejected",
    )


def test_candidate_artifact_must_match_manifest_exactly(tmp_path: Path) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)

    package = module.assert_manifest_artifact(_manifest(artifact), artifact)

    assert package["name"] == artifact.name
    assert artifact.sha256 == hashlib.sha256(b"exact candidate package").hexdigest()


def test_debian_amd64_identity_matches_native_package_graph(tmp_path: Path) -> None:
    module = _load_module()
    package = tmp_path / "Capsem_1.5.100_amd64.deb"
    package.write_bytes(b"exact linux candidate package")
    artifact = module.ArtifactIdentity.from_path(
        package,
        version="1.5.100",
        platform="linux",
        architecture="amd64",
    )
    manifest = _manifest(artifact)

    matched = module.assert_manifest_artifact(manifest, artifact)

    assert artifact.architecture is module.PackageArchitecture.AMD64
    assert matched["architecture"] == "amd64"


def test_existing_x86_64_manifest_package_resolves_without_mutating_authority(
    tmp_path: Path,
) -> None:
    module = _load_module()
    package = tmp_path / "Capsem_1.5.100_amd64.deb"
    payload = b"exact existing linux package"
    package.write_bytes(payload)
    document = {
        "packages": [
            {
                "name": package.name,
                "version": "1.5.100",
                "platform": "linux",
                "architecture": "x86_64",
                "bytes": len(payload),
                "digest": {"sha256": hashlib.sha256(payload).hexdigest()},
                "status": "current",
            }
        ],
        "profiles": {"code": {}},
    }
    contents = json.dumps(document, sort_keys=True).encode()

    artifact = module.artifact_identity_from_manifest_package(contents, package)

    assert artifact.architecture is module.PackageArchitecture.AMD64
    assert json.loads(contents) == document


@pytest.mark.parametrize("architecture", ["x86_64", "aarch64", ""])
def test_package_identity_rejects_machine_architectures_and_aliases(
    tmp_path: Path,
    architecture: str,
) -> None:
    module = _load_module()
    package = tmp_path / "Capsem_1.5.100_amd64.deb"
    package.write_bytes(b"exact linux candidate package")

    with pytest.raises(module.GlowupContractError, match="package architecture"):
        module.ArtifactIdentity.from_path(
            package,
            version="1.5.100",
            platform="linux",
            architecture=architecture,
        )


@pytest.mark.parametrize(
    ("name", "platform", "architecture"),
    [
        ("Capsem_1.5.100_x86_64.deb", "linux", "amd64"),
        ("Capsem_1.5.100_arm64.deb", "linux", "amd64"),
        ("Capsem_1.5.100_amd64.deb", "linux", "arm64"),
        ("Capsem-1.5.100.pkg", "macos", "amd64"),
    ],
)
def test_package_filename_platform_and_architecture_must_agree(
    tmp_path: Path,
    name: str,
    platform: str,
    architecture: str,
) -> None:
    module = _load_module()
    package = tmp_path / name
    package.write_bytes(b"candidate package")

    with pytest.raises(module.GlowupContractError, match=r"package|architecture"):
        module.ArtifactIdentity.from_path(
            package,
            version="1.5.100",
            platform=platform,
            architecture=architecture,
        )


@pytest.mark.parametrize(
    ("field", "bad_value"),
    [
        ("name", "Capsem-other.pkg"),
        ("version", "1.5.99"),
        ("platform", "linux"),
        ("architecture", "amd64"),
        ("bytes", 1),
        ("sha256", "0" * 64),
        ("status", "superseded"),
    ],
)
def test_candidate_artifact_rejects_every_identity_mismatch(
    tmp_path: Path,
    field: str,
    bad_value: object,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    manifest = _manifest(artifact)
    package = manifest["packages"][0]
    if field == "sha256":
        package["digest"]["sha256"] = bad_value
    else:
        package[field] = bad_value

    with pytest.raises(module.GlowupContractError, match=f"{field}|exactly one"):
        module.assert_manifest_artifact(manifest, artifact)


def test_candidate_artifact_rejects_ambiguous_package_records(tmp_path: Path) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    manifest = _manifest(artifact)
    manifest["packages"].append(dict(manifest["packages"][0]))

    with pytest.raises(module.GlowupContractError, match="exactly one"):
        module.assert_manifest_artifact(manifest, artifact)


def test_normalized_installed_evidence_is_platform_independent() -> None:
    module = _load_module()
    evidence = {
        "package_version": "1.5.100",
        "channel": "stable",
        "manifest_url": "file:///candidate/manifest.json",
        "package_receipt": True,
        "binary_cohort": True,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
        "profiles_ready": 3,
        "profiles_total": 3,
    }

    assert module.validate_installed_evidence(evidence) == evidence

    for field, bad_value in (
        ("package_receipt", False),
        ("binary_cohort", False),
        ("installed", False),
        ("running", False),
        ("service", "failed"),
        ("gateway", "failed"),
        ("profiles_ready", 2),
        ("profiles_total", 0),
    ):
        invalid = dict(evidence)
        invalid[field] = bad_value
        with pytest.raises(module.GlowupContractError, match=field):
            module.validate_installed_evidence(invalid)


def test_shared_report_has_one_schema_for_linux_and_macos(tmp_path: Path) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    evidence = {
        "package_version": artifact.version,
        "channel": "stable",
        "manifest_url": "file:///candidate/manifest.json",
        "package_receipt": True,
        "binary_cohort": True,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
        "profiles_ready": 3,
        "profiles_total": 3,
    }

    reports = [
        module.build_report(
            adapter=adapter,
            artifact=artifact,
            installed=evidence,
            capabilities=capabilities,
        )
        for adapter, capabilities in (
            ("linux-docker-systemd", {"native_install": True}),
            (
                "macos-tart-launchd",
                {"native_install": True, "physical_vz_boot": True},
            ),
        )
    ]

    assert {report["schema"] for report in reports} == {"capsem.release_glowup.v1"}
    assert {report["adapter"] for report in reports} == {
        "linux-docker-systemd",
        "macos-tart-launchd",
    }
    assert reports[0]["artifact"] == reports[1]["artifact"]
    json.dumps(reports)


def test_pairing_identity_is_derived_from_exact_manifest_and_package(
    tmp_path: Path,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    manifest = _manifest(artifact)
    manifest["profiles"] = {"code": {"revision": "profiles-1", "images": [{"digest": "a" * 64}]}}
    contents = json.dumps(manifest, sort_keys=True).encode()

    pairing = module.PairingIdentity.from_manifest_bytes(
        contents,
        artifact=artifact,
        channel="stable",
    )

    assert pairing.package_version == artifact.version
    assert pairing.package_sha256 == artifact.sha256
    assert pairing.manifest_sha256 == hashlib.sha256(contents).hexdigest()
    assert (
        pairing.profiles_sha256
        == hashlib.sha256(
            json.dumps(
                manifest["profiles"],
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
        ).hexdigest()
    )


def test_transition_sequence_proves_each_required_installed_state() -> None:
    module = _load_module()
    zero = "0" * 64
    one = "1" * 64
    two = "2" * 64
    three = "3" * 64
    four = "4" * 64
    five = "5" * 64
    initial = _pairing(
        module,
        channel="stable",
        manifest_sha256=zero,
        package_version="1.0.0",
        package_sha256=zero,
        profiles_sha256=zero,
    )
    binary = _pairing(
        module,
        channel="stable",
        manifest_sha256=one,
        package_version="1.1.0",
        package_sha256=one,
        profiles_sha256=zero,
    )
    profile = _pairing(
        module,
        channel="stable",
        manifest_sha256=two,
        package_version="1.1.0",
        package_sha256=one,
        profiles_sha256=two,
    )
    combined = _pairing(
        module,
        channel="stable",
        manifest_sha256=three,
        package_version="1.2.0",
        package_sha256=three,
        profiles_sha256=three,
    )
    nightly = _pairing(
        module,
        channel="nightly",
        manifest_sha256=four,
        package_version="1.3.0-nightly.1",
        package_sha256=four,
        profiles_sha256=five,
    )
    transitions = [
        _transition(
            module,
            module.TransitionKind.FRESH_INSTALL,
            before=None,
            after=initial,
        ),
        _transition(
            module,
            module.TransitionKind.BINARY_ONLY,
            before=initial,
            after=binary,
        ),
        _transition(
            module,
            module.TransitionKind.PROFILE_ONLY,
            before=binary,
            after=profile,
        ),
        _transition(
            module,
            module.TransitionKind.PROFILE_THEN_BINARY,
            before=profile,
            after=combined,
            staged_profiles_sha256=combined.profiles_sha256,
        ),
        _transition(
            module,
            module.TransitionKind.CHANNEL_SWITCH,
            before=combined,
            after=nightly,
        ),
        _transition(
            module,
            module.TransitionKind.TAMPER_REJECTION,
            before=nightly,
            after=nightly,
            result="rejected",
        ),
    ]

    report = module.validate_transition_sequence(transitions)

    assert [row["kind"] for row in report] == [kind.value for kind in module.TransitionKind]
    assert report[3]["staged_profiles_sha256"] == combined.profiles_sha256
    assert report[-1]["preserved_previous"] is True
    assert all(row["probes"] == {"doctor": True, "winterfell": True} for row in report)


def test_lane_scoped_transition_report_requires_declared_exact_order(
    tmp_path: Path,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    initial = _pairing(
        module,
        channel="nightly",
        manifest_sha256="0" * 64,
        package_version="1.5.99",
        package_sha256="0" * 64,
        profiles_sha256="2" * 64,
    )
    candidate = _pairing(
        module,
        channel="nightly",
        manifest_sha256="1" * 64,
        package_version=artifact.version,
        package_sha256=artifact.sha256,
        profiles_sha256="2" * 64,
    )
    fresh = _transition(
        module,
        module.TransitionKind.FRESH_INSTALL,
        before=None,
        after=initial,
    )
    binary = _transition(
        module,
        module.TransitionKind.BINARY_ONLY,
        before=initial,
        after=candidate,
    )
    installed = {
        "package_version": artifact.version,
        "channel": "nightly",
        "manifest_url": "https://release.test/assets/nightly/manifest.json",
        "package_receipt": True,
        "binary_cohort": True,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
        "profiles_ready": 1,
        "profiles_total": 1,
    }

    report = module.build_report(
        adapter="linux-docker-systemd",
        artifact=artifact,
        installed=installed,
        capabilities={"native_install": True},
        transitions=[fresh, binary],
        expected_transitions=[
            module.TransitionKind.FRESH_INSTALL,
            module.TransitionKind.BINARY_ONLY,
        ],
    )

    assert report["transition_scope"] == ["fresh_install", "binary_only"]
    assert [row["kind"] for row in report["transitions"]] == report["transition_scope"]

    for invalid_rows, invalid_scope in (
        ([binary, fresh], ["fresh_install", "binary_only"]),
        ([fresh], ["fresh_install", "binary_only"]),
        ([fresh, binary], ["fresh_install", "fresh_install"]),
        ([binary], ["binary_only"]),
    ):
        with pytest.raises(module.GlowupContractError, match="transition"):
            module.build_report(
                adapter="linux-docker-systemd",
                artifact=artifact,
                installed=installed,
                capabilities={"native_install": True},
                transitions=invalid_rows,
                expected_transitions=invalid_scope,
            )


@pytest.mark.parametrize(
    ("kind", "before_updates", "after_updates", "error"),
    [
        (
            "binary_only",
            {},
            {"profiles_sha256": "2" * 64},
            "profiles",
        ),
        (
            "profile_only",
            {},
            {"package_sha256": "2" * 64},
            "package",
        ),
        (
            "channel_switch",
            {},
            {"channel": "stable"},
            "channel",
        ),
    ],
)
def test_transition_contract_rejects_metadata_only_or_cross_family_changes(
    kind: str,
    before_updates: dict[str, str],
    after_updates: dict[str, str],
    error: str,
) -> None:
    module = _load_module()
    before_values = {
        "channel": "stable",
        "manifest_sha256": "0" * 64,
        "package_version": "1.0.0",
        "package_sha256": "0" * 64,
        "profiles_sha256": "0" * 64,
        **before_updates,
    }
    after_values = {
        "channel": "nightly" if kind == "channel_switch" else "stable",
        "manifest_sha256": "1" * 64,
        "package_version": "1.1.0" if kind == "binary_only" else "1.0.0",
        "package_sha256": "1" * 64 if kind == "binary_only" else "0" * 64,
        "profiles_sha256": "1" * 64 if kind == "profile_only" else "0" * 64,
        **after_updates,
    }

    with pytest.raises(module.GlowupContractError, match=error):
        _transition(
            module,
            module.TransitionKind(kind),
            before=_pairing(module, **before_values),
            after=_pairing(module, **after_values),
        )


def test_profile_then_binary_requires_exact_staged_profile_reuse() -> None:
    module = _load_module()
    before = _pairing(
        module,
        channel="stable",
        manifest_sha256="0" * 64,
        package_version="1.0.0",
        package_sha256="0" * 64,
        profiles_sha256="0" * 64,
    )
    after = _pairing(
        module,
        channel="stable",
        manifest_sha256="1" * 64,
        package_version="1.1.0",
        package_sha256="1" * 64,
        profiles_sha256="1" * 64,
    )

    with pytest.raises(module.GlowupContractError, match="staged profile"):
        _transition(
            module,
            module.TransitionKind.PROFILE_THEN_BINARY,
            before=before,
            after=after,
            staged_profiles_sha256="2" * 64,
        )


def test_tamper_rejection_requires_working_state_to_remain_exact() -> None:
    module = _load_module()
    before = _pairing(
        module,
        channel="stable",
        manifest_sha256="0" * 64,
        package_version="1.0.0",
        package_sha256="0" * 64,
        profiles_sha256="0" * 64,
    )
    changed = _pairing(
        module,
        channel="stable",
        manifest_sha256="1" * 64,
        package_version="1.0.0",
        package_sha256="0" * 64,
        profiles_sha256="0" * 64,
    )

    with pytest.raises(module.GlowupContractError, match="previous working"):
        _transition(
            module,
            module.TransitionKind.TAMPER_REJECTION,
            before=before,
            after=changed,
            result="rejected",
        )
    with pytest.raises(module.GlowupContractError, match="doctor"):
        module.build_transition_evidence(
            kind=module.TransitionKind.TAMPER_REJECTION,
            before=before,
            after=before,
            result="rejected",
            doctor_passed=False,
            winterfell_passed=True,
            preserved_previous=True,
        )


def test_transition_sequence_rejects_missing_duplicate_or_reordered_rows() -> None:
    module = _load_module()
    pairing = _pairing(
        module,
        channel="stable",
        manifest_sha256="0" * 64,
        package_version="1.0.0",
        package_sha256="0" * 64,
        profiles_sha256="0" * 64,
    )
    fresh = _transition(
        module,
        module.TransitionKind.FRESH_INSTALL,
        before=None,
        after=pairing,
    )

    for invalid in ([], [fresh], [fresh, fresh]):
        with pytest.raises(module.GlowupContractError, match="transition sequence"):
            module.validate_transition_sequence(invalid)


def test_exact_binary_pairing_uses_real_manifest_and_package_bytes(
    tmp_path: Path,
) -> None:
    module = _load_module()
    before_root = tmp_path / "before"
    before_root.mkdir()
    before_artifact = _artifact(before_root, module)
    after_path = tmp_path / "after" / "Capsem-1.5.101.pkg"
    after_path.parent.mkdir(parents=True)
    after_path.write_bytes(b"exact candidate package v2")
    after_artifact = module.ArtifactIdentity.from_path(
        after_path,
        version="1.5.101",
        platform="macos",
        architecture="arm64",
    )
    profiles = {"code": {"revision": "profiles-1"}}
    before_manifest = _manifest(before_artifact)
    before_manifest["channel"] = "stable"
    before_manifest["profiles"] = profiles
    after_manifest = _manifest(after_artifact)
    after_manifest["channel"] = "stable"
    after_manifest["profiles"] = profiles
    before_contents = json.dumps(before_manifest, sort_keys=True).encode()
    after_contents = json.dumps(after_manifest, sort_keys=True).encode()
    resolved_before = module.artifact_identity_from_manifest_package(
        before_contents,
        before_artifact.path,
    )
    resolved_after = module.artifact_identity_from_manifest_package(
        after_contents,
        after_artifact.path,
    )

    before, after = module.validate_pairing_inputs(
        kind=module.TransitionKind.BINARY_ONLY,
        channel="stable",
        before_manifest_bytes=before_contents,
        after_manifest_bytes=after_contents,
        before_artifact=resolved_before,
        after_artifact=resolved_after,
    )

    assert before.package_sha256 == before_artifact.sha256
    assert after.package_sha256 == after_artifact.sha256
    assert before.profiles_sha256 == after.profiles_sha256


def test_exact_profile_pairing_allows_only_the_selected_profile_to_change(
    tmp_path: Path,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    before_manifest = _manifest(artifact)
    before_manifest["channel"] = "nightly"
    before_manifest["profiles"] = {
        "code": {"revision": "code-1"},
        "experimental": {"revision": "experimental-1"},
    }
    after_manifest = json.loads(json.dumps(before_manifest))
    after_manifest["profiles"]["experimental"]["revision"] = "experimental-2"

    module.validate_pairing_inputs(
        kind=module.TransitionKind.PROFILE_ONLY,
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest, sort_keys=True).encode(),
        after_manifest_bytes=json.dumps(after_manifest, sort_keys=True).encode(),
        before_artifact=artifact,
        after_artifact=artifact,
        changed_profiles=("experimental",),
    )

    after_manifest["profiles"]["code"]["revision"] = "code-2"
    with pytest.raises(module.GlowupContractError, match="unselected profile"):
        module.validate_pairing_inputs(
            kind=module.TransitionKind.PROFILE_ONLY,
            channel="nightly",
            before_manifest_bytes=json.dumps(before_manifest, sort_keys=True).encode(),
            after_manifest_bytes=json.dumps(after_manifest, sort_keys=True).encode(),
            before_artifact=artifact,
            after_artifact=artifact,
            changed_profiles=("experimental",),
        )


def test_exact_profile_pairing_allows_first_profile_in_empty_channel(
    tmp_path: Path,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    before_manifest = _manifest(artifact)
    before_manifest["channel"] = "nightly"
    before_manifest["profiles"] = {}
    after_manifest = json.loads(json.dumps(before_manifest))
    after_manifest["profiles"] = {"code": {"revision": "code-1"}}

    before, after = module.validate_pairing_inputs(
        kind=module.TransitionKind.PROFILE_ONLY,
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest, sort_keys=True).encode(),
        after_manifest_bytes=json.dumps(after_manifest, sort_keys=True).encode(),
        before_artifact=artifact,
        after_artifact=artifact,
        changed_profiles=("code",),
    )

    assert before.package_sha256 == after.package_sha256
    assert before.profiles_sha256 != after.profiles_sha256


def test_exact_pairing_classifier_distinguishes_binary_and_staged_profile(
    tmp_path: Path,
) -> None:
    module = _load_module()
    classifier = _load_first_release()
    before_root = tmp_path / "before"
    before_root.mkdir()
    before_artifact = _artifact(before_root, module)
    after_path = tmp_path / "after" / "Capsem-1.5.101.pkg"
    after_path.parent.mkdir(parents=True)
    after_path.write_bytes(b"exact candidate package v2")
    after_artifact = module.ArtifactIdentity.from_path(
        after_path,
        version="1.5.101",
        platform="macos",
        architecture="arm64",
    )
    before_manifest = _manifest(before_artifact)
    before_manifest["channel"] = "nightly"
    before_manifest["profiles"] = {
        "code": {"revision": "code-1"},
        "experimental": {"revision": "experimental-1"},
    }
    after_manifest = _manifest(after_artifact)
    after_manifest["channel"] = "nightly"
    after_manifest["profiles"] = json.loads(json.dumps(before_manifest["profiles"]))

    kind, profile = classifier.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is classifier.TransitionKind.BINARY_ONLY
    assert profile == ()

    after_manifest["profiles"]["experimental"]["revision"] = "experimental-2"
    kind, profile = classifier.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is classifier.TransitionKind.PROFILE_THEN_BINARY
    assert profile == ("experimental",)

    after_manifest["profiles"]["code"]["revision"] = "code-2"
    kind, profiles = classifier.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is classifier.TransitionKind.PROFILE_THEN_BINARY
    assert profiles == ("code", "experimental")


def test_exact_pairing_classifier_anchors_nightly_on_verified_stable(
    tmp_path: Path,
) -> None:
    module = _load_module()
    classifier = _load_first_release()
    before_root = tmp_path / "before"
    before_root.mkdir()
    before_artifact = _artifact(before_root, module)
    after_path = tmp_path / "after" / "Capsem-1.5.101.pkg"
    after_path.parent.mkdir(parents=True)
    after_path.write_bytes(b"exact nightly candidate")
    after_artifact = module.ArtifactIdentity.from_path(
        after_path,
        version="1.5.101",
        platform="macos",
        architecture="arm64",
    )
    before_manifest = _manifest(before_artifact)
    before_manifest["channel"] = "stable"
    before_manifest["profiles"] = {
        "code": {"revision": "code-stable"},
        "co-work": {"revision": "co-work-stable"},
    }
    after_manifest = _manifest(after_artifact)
    after_manifest["channel"] = "nightly"
    after_manifest["profiles"] = json.loads(json.dumps(before_manifest["profiles"]))

    kind, profiles = classifier.classify_pairing_inputs(
        channel="nightly",
        baseline_channel="stable",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )

    assert kind is classifier.TransitionKind.CHANNEL_SWITCH
    assert profiles == ("co-work", "code")

    before_manifest["channel"] = "nightly"
    with pytest.raises(classifier.GlowupContractError, match="baseline channel"):
        classifier.classify_pairing_inputs(
            channel="nightly",
            baseline_channel="stable",
            before_manifest_bytes=json.dumps(before_manifest).encode(),
            after_manifest_bytes=json.dumps(after_manifest).encode(),
            before_artifact=before_artifact,
            after_artifact=after_artifact,
        )


def test_only_cross_channel_pairing_requires_an_explicit_product_switch() -> None:
    module = _load_module()

    assert module.explicit_channel_switch_args(
        module.TransitionKind.CHANNEL_SWITCH, "nightly"
    ) == ("update", "--yes", "--channel", "nightly")
    assert module.explicit_channel_switch_args(
        module.TransitionKind.BINARY_ONLY, "stable"
    ) == ()


def test_cross_channel_profile_release_stages_the_complete_target_but_owns_one_profile() -> None:
    module = _load_local_glowup()

    module.validate_selected_profile_scope(
        transition=module.TransitionKind.CHANNEL_SWITCH,
        selected_profile="code",
        changed_profiles=("co-work", "code"),
    )
    with pytest.raises(SystemExit, match="selected profile"):
        module.validate_selected_profile_scope(
            transition=module.TransitionKind.CHANNEL_SWITCH,
            selected_profile="code",
            changed_profiles=("co-work",),
        )
    with pytest.raises(SystemExit, match="manifest delta"):
        module.validate_selected_profile_scope(
            transition=module.TransitionKind.PROFILE_ONLY,
            selected_profile="code",
            changed_profiles=("co-work", "code"),
        )


def test_exact_pairing_rejects_manifest_channel_or_package_mismatch(
    tmp_path: Path,
) -> None:
    module = _load_module()
    artifact = _artifact(tmp_path, module)
    manifest = _manifest(artifact)
    manifest["channel"] = "stable"
    contents = json.dumps(manifest, sort_keys=True).encode()

    with pytest.raises(module.GlowupContractError, match="channel"):
        module.validate_pairing_inputs(
            kind=module.TransitionKind.BINARY_ONLY,
            channel="nightly",
            before_manifest_bytes=contents,
            after_manifest_bytes=contents,
            before_artifact=artifact,
            after_artifact=artifact,
        )

    changed = tmp_path / artifact.name
    changed.write_bytes(b"different package bytes")
    mismatched = module.ArtifactIdentity.from_path(
        changed,
        version=artifact.version,
        platform=artifact.platform,
        architecture=artifact.architecture.value,
    )
    with pytest.raises(module.GlowupContractError, match="sha256"):
        module.validate_pairing_inputs(
            kind=module.TransitionKind.PROFILE_ONLY,
            channel="stable",
            before_manifest_bytes=contents,
            after_manifest_bytes=contents,
            before_artifact=mismatched,
            after_artifact=mismatched,
            changed_profiles=("work",),
        )


def test_release_pairing_cli_is_all_or_nothing() -> None:
    module = _load_local_glowup()
    empty = SimpleNamespace(
        release_channel=None,
        release_baseline_channel=None,
        release_transition=None,
        before_manifest=None,
        after_manifest=None,
        before_package=None,
        before_profile_inputs=None,
        after_profile_inputs=None,
        profile=None,
        candidate_profile_publication=None,
        publication_base=None,
        input_deb=Path("candidate.deb"),
    )
    assert module.validate_exact_release_pairing(empty) is None

    partial = SimpleNamespace(**vars(empty))
    partial.release_channel = "stable"
    with pytest.raises(SystemExit, match="exact pairing requires"):
        module.validate_exact_release_pairing(partial)

    # Cleared, not absent. The channel-switch run is *given* these variables as
    # empty strings -- that is how it is made to rediscover the channel from the
    # installed system instead of inheriting what the previous run was told. An
    # empty string that counts as present makes the whole set look half-supplied
    # and the run dies before it starts, which is what happened: the rehearsal's
    # channel-switch step failed with all four manifests "missing" while the
    # environment had deliberately emptied every one of them.
    cleared = SimpleNamespace(**vars(empty))
    cleared.release_channel = ""
    cleared.release_transition = ""
    cleared.profile = ""
    cleared.publication_base = ""
    assert module.validate_exact_release_pairing(cleared) is None


def test_local_channel_import_uses_the_typed_selected_revision_policy() -> None:
    """Public legacy revisions are imported, never accepted for new authoring."""
    tree = ast.parse(LOCAL_GLOWUP_PATH.read_text(encoding="utf-8"))
    authoring_partials = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and isinstance(node.func.value, ast.Name)
        and node.func.value.id == "functools"
        and node.func.attr == "partial"
        and node.args
        and isinstance(node.args[0], ast.Name)
        and node.args[0].id == "author_native_candidate"
    ]

    assert len(authoring_partials) == 1
    policy = next(
        keyword.value
        for keyword in authoring_partials[0].keywords
        if keyword.arg == "profile_revision_policy"
    )
    assert isinstance(policy, ast.Attribute)
    assert isinstance(policy.value, ast.Name)
    assert (policy.value.id, policy.attr) == ("args", "profile_revision_policy")


def test_local_glowup_exports_bounded_started_evidence_before_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _load_local_glowup()
    evidence = tmp_path / "evidence"
    package = tmp_path / "Capsem_0.6.0_amd64.deb"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            str(LOCAL_GLOWUP_PATH),
            "--input-deb",
            str(package),
            "--source-commit",
            "0" * 40,
            "--bin-dir",
            str(tmp_path / "bin"),
            "--assets-dir",
            str(tmp_path / "assets"),
            "--config-root",
            str(tmp_path / "config"),
            "--work-dir",
            str(tmp_path / "work"),
            "--evidence-dir",
            str(evidence),
            "--profile-revision-policy",
            "selected-input",
        ],
    )

    def fail(_args) -> None:
        raise SystemExit("forced failure")

    monkeypatch.setattr(module, "validate_exact_release_pairing", fail)

    with pytest.raises(SystemExit, match="forced failure"):
        module.main()

    assert json.loads((evidence / "started.json").read_text()) == {
        "schema": "capsem.glowup.run.v1",
        "package": package.name,
    }
    assert [path.name for path in evidence.iterdir()] == ["started.json"]


def test_exact_release_transport_changes_only_urls_and_reuses_exact_bytes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _load_local_glowup()
    before_root = tmp_path / "before"
    after_root = tmp_path / "after"
    before_root.mkdir()
    after_root.mkdir()
    before_package = before_root / "Capsem-1.5.100.pkg"
    after_package = after_root / "Capsem-1.5.101.pkg"
    before_package.write_bytes(b"before package")
    after_package.write_bytes(b"after package")
    before_artifact = module.ArtifactIdentity.from_path(
        before_package,
        version="1.5.100",
        platform="macos",
        architecture="arm64",
    )
    after_artifact = module.ArtifactIdentity.from_path(
        after_package,
        version="1.5.101",
        platform="macos",
        architecture="arm64",
    )
    before_profile_url = "https://profiles.test/code-1/rootfs.erofs"
    after_profile_url = "https://profiles.test/code-2/rootfs.erofs"

    def authority(artifact, package_url: str, profile_url: str, revision: str):
        manifest = _manifest(artifact)
        manifest["channel"] = "nightly"
        manifest["packages"][0]["url"] = package_url
        manifest["profiles"] = {
            "code": {
                "revision": revision,
                "architectures": [
                    {
                        "architecture": "arm64",
                        "images": [
                            {
                                "kind": "rootfs",
                                "url": profile_url,
                                "bytes": 13,
                                "digest": {
                                    "sha256": "a" * 64,
                                    "blake3": "b" * 64,
                                },
                            }
                        ],
                    }
                ],
            }
        }
        return manifest

    before_document = authority(
        before_artifact,
        "https://packages.test/Capsem-1.5.100.pkg",
        before_profile_url,
        "code-1",
    )
    after_document = authority(
        after_artifact,
        "https://packages.test/Capsem-1.5.101.pkg",
        after_profile_url,
        "code-2",
    )
    before_manifest = before_root / "manifest.json"
    after_manifest = after_root / "manifest.json"
    before_manifest.write_text(json.dumps(before_document, sort_keys=True))
    after_manifest.write_text(json.dumps(after_document, sort_keys=True))
    before_inputs = before_root / "profile-inputs"
    after_inputs = after_root / "profile-inputs"
    profile_relative = Path("profiles/code/arm64/images/rootfs.erofs")
    for inputs, payload in (
        (before_inputs, b"before rootfs"),
        (after_inputs, b"after rootfs"),
    ):
        path = inputs / profile_relative
        path.parent.mkdir(parents=True)
        path.write_bytes(payload)

    reports = {
        before_inputs: (
            {
                "kind": "profiles",
                "manifest_url": "https://release.test/assets/nightly/manifest.json",
                "artifacts": [{"url": before_profile_url, "path": profile_relative.as_posix()}],
            },
            before_document,
            {},
        ),
        after_inputs: (
            {
                "kind": "profiles",
                "manifest_url": after_manifest.resolve().as_uri(),
                "artifacts": [{"url": after_profile_url, "path": profile_relative.as_posix()}],
            },
            after_document,
            {},
        ),
    }
    monkeypatch.setattr(
        module,
        "load_verified_release_inputs",
        lambda input_dir: reports[input_dir],
    )
    before, after = module.validate_pairing_inputs(
        kind=module.TransitionKind.PROFILE_THEN_BINARY,
        channel="nightly",
        before_manifest_bytes=before_manifest.read_bytes(),
        after_manifest_bytes=after_manifest.read_bytes(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
        changed_profiles=("code",),
    )
    pairing = module.ExactReleasePairing(
        channel="nightly",
        baseline_channel="nightly",
        transition=module.TransitionKind.PROFILE_THEN_BINARY,
        changed_profiles=("code",),
        before=before,
        after=after,
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        before_package=before_package,
        after_package=after_package,
        before_profile_inputs=before_inputs,
        after_profile_inputs=after_inputs,
    )
    before_authority_bytes = before_manifest.read_bytes()
    after_authority_bytes = after_manifest.read_bytes()

    transport = module.stage_exact_release_transport(
        pairing,
        dist=tmp_path / "dist",
        base_url="http://127.0.0.1:8765",
    )

    assert transport.current_manifest.read_bytes() == transport.before_manifest.read_bytes()
    assert transport.current_manifest_url == (
        "http://127.0.0.1:8765/transitions/assets/nightly/manifest.json"
    )
    channel_catalog = json.loads(transport.channel_catalog.read_text())
    selected = channel_catalog["channels"]["nightly"]["manifests"]
    assert len(selected) == 1
    assert selected[0]["status"] == "current"
    assert selected[0]["url"] == "/transitions/assets/nightly/manifest.json"
    assert (
        selected[0]["digest"]["sha256"]
        == hashlib.sha256(transport.before_manifest.read_bytes()).hexdigest()
    )

    assert transport.before_package.read_bytes() == before_package.read_bytes()
    assert transport.after_package.read_bytes() == after_package.read_bytes()
    assert before_manifest.read_bytes() == before_authority_bytes
    assert after_manifest.read_bytes() == after_authority_bytes
    projected = json.loads(transport.after_manifest.read_text())
    assert projected["packages"][0]["digest"] == after_document["packages"][0]["digest"]
    assert projected["profiles"]["code"]["revision"] == "code-2"
    assert projected["packages"][0]["url"].startswith("http://127.0.0.1:8765/transitions/after/")
    assert projected["profiles"]["code"]["architectures"][0]["images"][0]["url"].startswith(
        "http://127.0.0.1:8765/transitions/after/"
    )

    module.promote_exact_candidate_transport(transport)

    assert transport.current_manifest.read_bytes() == transport.after_manifest.read_bytes()
    assert not transport.current_manifest.with_suffix(".next").exists()
    promoted_catalog = json.loads(transport.channel_catalog.read_text())
    promoted_record = promoted_catalog["channels"]["nightly"]["manifests"][0]
    assert (
        promoted_record["digest"]["sha256"]
        == hashlib.sha256(transport.after_manifest.read_bytes()).hexdigest()
    )

    candidates = module.stage_adversarial_exact_candidates(
        pairing,
        transport,
        output_dir=tmp_path / "adversarial",
        architecture="arm64",
    )

    assert after_manifest.read_bytes() == after_authority_bytes
    assert (
        transport.after_manifest.read_text()
        == json.dumps(projected, indent=2, sort_keys=True) + "\n"
    )
    tampered = json.loads(candidates.tampered_manifest.read_text())
    incompatible = json.loads(candidates.incompatible_manifest.read_text())
    assert (
        tampered["profiles"]["code"]["architectures"][0]["images"][0]["digest"]["sha256"]
        == "0" * 64
    )
    assert (
        tampered["profiles"]["code"]["architectures"][0]["images"][0]["digest"]["blake3"]
        == "0" * 64
    )
    assert incompatible["profiles"]["code"]["min_capsem_version"] == "9999.0.0"
    assert candidates.tampered_manifest.read_bytes() != transport.after_manifest.read_bytes()
    assert candidates.incompatible_manifest.read_bytes() != transport.after_manifest.read_bytes()


@pytest.mark.parametrize(
    "fail_call",
    (
        pytest.param(None, id="success"),
        pytest.param(2, id="failure"),
    ),
)
def test_exact_installed_glowup_uses_service_poll_and_probes_each_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    fail_call: int | None,
) -> None:
    module = _load_local_glowup()
    before_package = tmp_path / "Capsem_1.5.99_amd64.deb"
    after_package = tmp_path / "Capsem_1.5.100_amd64.deb"
    before_package.write_bytes(b"before exact deb")
    after_package.write_bytes(b"after exact deb")
    before_artifact = module.ArtifactIdentity.from_path(
        before_package,
        version="1.5.99",
        platform="linux",
        architecture="amd64",
    )
    after_artifact = module.ArtifactIdentity.from_path(
        after_package,
        version="1.5.100",
        platform="linux",
        architecture="amd64",
    )
    before_document = _manifest(before_artifact)
    before_document["channel"] = "nightly"
    after_document = _manifest(after_artifact)
    after_document["channel"] = "nightly"
    for document in (before_document, after_document):
        document["profiles"] = {
            "work": {
                "revision": "work-1",
                "architectures": [
                    {
                        "architecture": "amd64",
                        "images": [
                            {
                                "kind": "rootfs",
                                "url": "http://127.0.0.1:8765/rootfs.erofs",
                                "bytes": 13,
                                "digest": {
                                    "sha256": "a" * 64,
                                    "blake3": "b" * 64,
                                },
                                "status": "current",
                            }
                        ],
                    }
                ],
            }
        }
    before_manifest = tmp_path / "before.json"
    after_manifest = tmp_path / "after.json"
    current_manifest = tmp_path / "current" / "manifest.json"
    current_manifest.parent.mkdir()
    before_manifest.write_text(json.dumps(before_document, sort_keys=True))
    after_manifest.write_text(json.dumps(after_document, sort_keys=True))
    current_manifest.write_bytes(before_manifest.read_bytes())
    channel_catalog = tmp_path / "channels.json"
    channel_catalog.write_text(
        json.dumps(
            {
                "channels": {
                    "nightly": {
                        "manifests": [
                            {
                                "version": "1.5.100",
                                "status": "current",
                                "url": "/transitions/assets/nightly/manifest.json",
                                "digest": {
                                    "sha256": hashlib.sha256(
                                        before_manifest.read_bytes()
                                    ).hexdigest(),
                                    "blake3": "a" * 64,
                                },
                            }
                        ]
                    }
                }
            }
        )
    )
    before = module.PairingIdentity.from_manifest_bytes(
        before_manifest.read_bytes(),
        artifact=before_artifact,
        channel="nightly",
    )
    after = module.PairingIdentity.from_manifest_bytes(
        after_manifest.read_bytes(),
        artifact=after_artifact,
        channel="nightly",
    )
    pairing = module.ExactReleasePairing(
        channel="nightly",
        baseline_channel="nightly",
        transition=module.TransitionKind.BINARY_ONLY,
        changed_profiles=(),
        before=before,
        after=after,
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        before_package=before_package,
        after_package=after_package,
        before_profile_inputs=tmp_path / "before-profiles",
        after_profile_inputs=tmp_path / "after-profiles",
    )
    transport = module.ExactReleaseTransport(
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        current_manifest=current_manifest,
        channel_catalog=channel_catalog,
        current_manifest_url=("http://127.0.0.1:8765/transitions/assets/nightly/manifest.json"),
        before_manifest_url=("http://127.0.0.1:8765/transitions/assets/nightly/manifest.json"),
        current_manifest_route="/transitions/assets/nightly/manifest.json",
        channel_catalog_url="http://127.0.0.1:8765/transitions/channels.json",
        before_package=before_package,
        after_package=after_package,
    )
    calls: list[list[str]] = []

    def capture(command: list[str], **_kwargs) -> None:
        calls.append(command)
        if len(calls) == fail_call:
            raise RuntimeError("forced installed transition failure")

    monkeypatch.setattr(module, "run", capture)
    arguments = {
        "pairing": pairing,
        "transport": transport,
        "install_script_url": "http://127.0.0.1:8765/install.sh",
        "release_base_url": "http://127.0.0.1:8765",
        "evidence_dir": tmp_path / "evidence",
    }
    if fail_call is not None:
        with pytest.raises(RuntimeError, match="forced installed transition failure"):
            module.run_exact_installed_glowup(**arguments)
        assert calls[-1] == AUTOMATIC_UPDATE_POLL_CLEANUP
        return

    evidence = module.run_exact_installed_glowup(**arguments)

    assert len(calls) == 6
    assert calls[-1] == AUTOMATIC_UPDATE_POLL_CLEANUP
    before_script = calls[0][-1]
    after_script = calls[1][-1]
    tamper_script = calls[2][-1]
    incompatible_script = calls[3][-1]
    preserved_script = calls[4][-1]
    for script in (
        before_script,
        after_script,
        tamper_script,
        incompatible_script,
        preserved_script,
    ):
        subprocess.run(["bash", "-n"], input=script, text=True, check=True)
    assert "CAPSEM_MANIFEST_URL=" in before_script
    assert "update --assets --channel" in before_script
    assert "systemctl --user set-environment" in before_script
    assert "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS=2" in before_script
    assert "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS=2" in before_script
    assert "probe_installed_transition fresh-install" in before_script
    assert "observe_update_transition binary_only activated" in after_script
    assert "release_transition.py" in after_script
    assert "probe_installed_transition candidate-after" in after_script
    assert "observe_update_transition tampered_artifact rejected" in tamper_script
    assert "automatic release update failed" not in tamper_script
    assert "tampered-before-manifest.json" in tamper_script
    assert "tampered-rejection.json" in tamper_script
    assert "observe_update_transition incompatible_profile rejected" in incompatible_script
    assert "incompatible-before-manifest.json" in incompatible_script
    assert "incompatible-rejection.json" in incompatible_script
    assert "probe_installed_transition rejection-preserved" in preserved_script
    for script in (before_script, after_script, preserved_script):
        assert "scripts/verify-installed-release.py" in script
        assert '"$CAPSEM_BIN" doctor' in script
        assert "scripts/run-installed-winterfell.py" in script
        assert "capsem-mock-server" in script
        assert "update --yes" not in script
    assert "update --yes" not in tamper_script
    assert "update --yes" not in incompatible_script
    assert current_manifest.read_bytes() == after_manifest.read_bytes()
    assert evidence.fresh_installed.name == "fresh-install-installed.json"
    assert evidence.fresh_transition.name == "fresh-install-transition.json"
    assert evidence.candidate_transition.name == "candidate-after-transition.json"
    assert evidence.candidate_winterfell.name == "candidate-after-winterfell.json"
    assert evidence.tamper_rejection.name == "tampered-rejection.json"
    assert evidence.incompatible_rejection.name == "incompatible-rejection.json"
    assert evidence.preserved_installed.name == "rejection-preserved-installed.json"
    assert evidence.preserved_doctor.name == "rejection-preserved-doctor.json"
    assert evidence.preserved_winterfell.name == "rejection-preserved-winterfell.json"


def test_first_profile_glowup_never_installs_empty_authoring_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _load_local_glowup()
    package = tmp_path / "Capsem_1.5.100_amd64.deb"
    package.write_bytes(b"existing exact deb")
    artifact = module.ArtifactIdentity.from_path(
        package,
        version="1.5.100",
        platform="linux",
        architecture="amd64",
    )
    before_document = _manifest(artifact)
    before_document.update({"channel": "nightly", "profiles": {}})
    after_document = _manifest(artifact)
    after_document.update(
        {
            "channel": "nightly",
            "profiles": {
                "code": {
                    "revision": "code-1",
                    "architectures": [
                        {
                            "architecture": "x86_64",
                            "images": [
                                {
                                    "kind": "rootfs",
                                    "url": "https://example.test/rootfs.erofs",
                                    "bytes": 13,
                                    "digest": {
                                        "sha256": "a" * 64,
                                        "blake3": "b" * 64,
                                    },
                                    "status": "current",
                                }
                            ],
                        }
                    ],
                }
            },
        }
    )
    before_manifest = tmp_path / "before.json"
    after_manifest = tmp_path / "after.json"
    current_manifest = tmp_path / "current/manifest.json"
    current_manifest.parent.mkdir()
    before_manifest.write_text(json.dumps(before_document, sort_keys=True))
    after_manifest.write_text(json.dumps(after_document, sort_keys=True))
    current_manifest.write_bytes(before_manifest.read_bytes())
    channel_catalog = tmp_path / "channels.json"
    channel_catalog.write_text(
        json.dumps(
            {
                "channels": {
                    "nightly": {
                        "manifests": [
                            {
                                "version": "1.5.100",
                                "status": "current",
                                "url": "/transitions/assets/nightly/manifest.json",
                                "digest": {
                                    "sha256": hashlib.sha256(
                                        before_manifest.read_bytes()
                                    ).hexdigest(),
                                    "blake3": "a" * 64,
                                },
                            }
                        ]
                    }
                }
            }
        )
    )
    before = module.PairingIdentity.from_manifest_bytes(
        before_manifest.read_bytes(),
        artifact=artifact,
        channel="nightly",
        allow_empty_profiles=True,
    )
    after = module.PairingIdentity.from_manifest_bytes(
        after_manifest.read_bytes(),
        artifact=artifact,
        channel="nightly",
    )
    pairing = module.ExactReleasePairing(
        channel="nightly",
        baseline_channel="nightly",
        transition=module.TransitionKind.PROFILE_ONLY,
        changed_profiles=("code",),
        before=before,
        after=after,
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        before_package=package,
        after_package=package,
        before_profile_inputs=tmp_path / "before-profiles",
        after_profile_inputs=tmp_path / "after-profiles",
    )
    transport = module.ExactReleaseTransport(
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        current_manifest=current_manifest,
        channel_catalog=channel_catalog,
        current_manifest_url=("http://127.0.0.1:8765/transitions/assets/nightly/manifest.json"),
        before_manifest_url=("http://127.0.0.1:8765/transitions/assets/nightly/manifest.json"),
        current_manifest_route="/transitions/assets/nightly/manifest.json",
        channel_catalog_url="http://127.0.0.1:8765/transitions/channels.json",
        before_package=package,
        after_package=package,
    )
    calls: list[list[str]] = []
    monkeypatch.setattr(module, "run", lambda command, **_kwargs: calls.append(command))

    evidence = module.run_exact_installed_glowup(
        pairing=pairing,
        transport=transport,
        install_script_url="http://127.0.0.1:8765/install.sh",
        release_base_url="http://127.0.0.1:8765",
        evidence_dir=tmp_path / "evidence",
    )

    assert len(calls) == 5
    assert calls[-1] == AUTOMATIC_UPDATE_POLL_CLEANUP
    fresh_script = calls[0][-1]
    assert "probe_installed_transition fresh-install" in fresh_script
    assert "probe_installed_transition candidate-after" not in fresh_script
    # This asserted that `transport.before_manifest_url` was absent. That field
    # was never read anywhere -- the manifests are promoted by copying files,
    # not by switching URLs -- so the assertion checked that a string appearing
    # nowhere appeared nowhere. What it meant to pin is that the fresh install
    # polls the one URL the service is configured with.
    assert transport.current_manifest_url in fresh_script
    assert fresh_script.index("CAPSEM_MANIFEST_URL=") < fresh_script.index(
        "probe_installed_transition fresh-install"
    )
    assert evidence.fresh_uses_after is True
    installed = {
        "package_version": after.package_version,
        "channel": "nightly",
        "manifest_url": transport.current_manifest_url,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
        "profiles_ready": 1,
        "profiles_total": 1,
    }
    for path in (evidence.fresh_installed, evidence.preserved_installed):
        path.write_text(json.dumps(installed))
    for path, schema in (
        (evidence.fresh_doctor, "capsem.installed_doctor.v1"),
        (evidence.fresh_winterfell, "capsem.installed_winterfell.v1"),
        (evidence.preserved_doctor, "capsem.installed_doctor.v1"),
        (evidence.preserved_winterfell, "capsem.installed_winterfell.v1"),
    ):
        path.write_text(json.dumps({"schema": schema, "passed": True}))
    after_sha256 = hashlib.sha256(transport.after_manifest.read_bytes()).hexdigest()
    evidence.fresh_transition.write_text(
        json.dumps(
            _transition_verdict(
                kind="fresh_install",
                result="activated",
                source=transport.current_manifest_url,
                candidate_sha256=after_sha256,
                installed_sha256=after_sha256,
            )
        )
    )
    for index, (path, kind) in enumerate(
        (
            (evidence.tamper_rejection, "tampered_artifact"),
            (evidence.incompatible_rejection, "incompatible_profile"),
        ),
        start=3,
    ):
        path.write_text(
            json.dumps(
                _transition_verdict(
                    kind=kind,
                    result="rejected",
                    source=transport.current_manifest_url,
                    candidate_sha256=str(index) * 64,
                    installed_sha256=after_sha256,
                    previous_sha256=after_sha256,
                )
            )
        )

    rows = module.exact_installed_transition_rows(pairing, transport, evidence)

    assert [row["kind"] for row in rows] == ["fresh_install", "tamper_rejection"]
    assert rows[0]["after"] == after.as_report()
    assert rows[-1]["preserved_previous"] is True


def test_exact_installed_transition_rows_require_real_probe_reports(tmp_path: Path) -> None:
    module = _load_local_glowup()
    before = _pairing(
        module,
        channel="nightly",
        manifest_sha256="0" * 64,
        package_version="1.5.99",
        package_sha256="0" * 64,
        profiles_sha256="2" * 64,
    )
    after = _pairing(
        module,
        channel="nightly",
        manifest_sha256="1" * 64,
        package_version="1.5.100",
        package_sha256="1" * 64,
        profiles_sha256="2" * 64,
    )
    pairing = SimpleNamespace(
        transition=module.TransitionKind.BINARY_ONLY,
        before=before,
        after=after,
    )
    before_manifest = tmp_path / "before-manifest.json"
    after_manifest = tmp_path / "after-manifest.json"
    before_manifest.write_text('{"state":"before"}')
    after_manifest.write_text('{"state":"after"}')
    source = "https://release.test/assets/nightly/manifest.json"
    transport = SimpleNamespace(
        before_manifest=before_manifest,
        after_manifest=after_manifest,
        current_manifest_url=source,
    )
    evidence = module.ExactInstalledGlowupEvidence(
        fresh_transition=tmp_path / "fresh-transition.json",
        fresh_installed=tmp_path / "fresh-installed.json",
        fresh_doctor=tmp_path / "fresh-doctor.json",
        fresh_winterfell=tmp_path / "fresh-winterfell.json",
        candidate_installed=tmp_path / "candidate-installed.json",
        candidate_doctor=tmp_path / "candidate-doctor.json",
        candidate_winterfell=tmp_path / "candidate-winterfell.json",
        candidate_transition=tmp_path / "candidate-transition.json",
        tamper_rejection=tmp_path / "tampered-rejection.json",
        incompatible_rejection=tmp_path / "incompatible-rejection.json",
        preserved_installed=tmp_path / "preserved-installed.json",
        preserved_doctor=tmp_path / "preserved-doctor.json",
        preserved_winterfell=tmp_path / "preserved-winterfell.json",
    )
    for path, package_version in (
        (evidence.fresh_installed, before.package_version),
        (evidence.candidate_installed, after.package_version),
        (evidence.preserved_installed, after.package_version),
    ):
        path.write_text(
            json.dumps(
                {
                    "package_version": package_version,
                    "channel": "nightly",
                    "manifest_url": "https://release.test/assets/nightly/manifest.json",
                    "installed": True,
                    "running": True,
                    "service": "ok",
                    "gateway": "ok",
                    "profiles_ready": 1,
                    "profiles_total": 1,
                }
            )
        )
    for path in (
        evidence.fresh_doctor,
        evidence.candidate_doctor,
        evidence.preserved_doctor,
    ):
        path.write_text(json.dumps({"schema": "capsem.installed_doctor.v1", "passed": True}))
    for path in (
        evidence.fresh_winterfell,
        evidence.candidate_winterfell,
        evidence.preserved_winterfell,
    ):
        path.write_text(json.dumps({"schema": "capsem.installed_winterfell.v1", "passed": True}))
    before_sha256 = hashlib.sha256(before_manifest.read_bytes()).hexdigest()
    after_sha256 = hashlib.sha256(after_manifest.read_bytes()).hexdigest()
    evidence.fresh_transition.write_text(
        json.dumps(
            _transition_verdict(
                kind="fresh_install",
                result="activated",
                source=source,
                candidate_sha256=before_sha256,
                installed_sha256=before_sha256,
            )
        )
    )
    evidence.candidate_transition.write_text(
        json.dumps(
            _transition_verdict(
                kind="binary_only",
                result="activated",
                source=source,
                candidate_sha256=after_sha256,
                installed_sha256=after_sha256,
            )
        )
    )
    evidence.tamper_rejection.write_text(
        json.dumps(
            _transition_verdict(
                kind="tampered_artifact",
                result="rejected",
                source=source,
                candidate_sha256="3" * 64,
                installed_sha256=after_sha256,
                previous_sha256=after_sha256,
            )
        )
    )
    evidence.incompatible_rejection.write_text(
        json.dumps(
            _transition_verdict(
                kind="incompatible_profile",
                result="rejected",
                source=source,
                candidate_sha256="4" * 64,
                installed_sha256=after_sha256,
                previous_sha256=after_sha256,
            )
        )
    )

    rows = module.exact_installed_transition_rows(pairing, transport, evidence)

    assert [row["kind"] for row in rows] == [
        "fresh_install",
        "binary_only",
        "tamper_rejection",
    ]
    assert all(row["probes"] == {"doctor": True, "winterfell": True} for row in rows)
    assert rows[-1]["before"] == rows[-1]["after"]
    assert rows[-1]["preserved_previous"] is True

    evidence.tamper_rejection.write_text(
        json.dumps(
            _transition_verdict(
                kind="tampered_artifact",
                result="activated",
                source=source,
                candidate_sha256="3" * 64,
                installed_sha256="3" * 64,
            )
        )
    )
    with pytest.raises(SystemExit, match="transition verdict failed"):
        module.exact_installed_transition_rows(pairing, transport, evidence)
    evidence.tamper_rejection.write_text(
        json.dumps(
            _transition_verdict(
                kind="tampered_artifact",
                result="rejected",
                source=source,
                candidate_sha256="3" * 64,
                installed_sha256=after_sha256,
                previous_sha256=after_sha256,
            )
        )
    )
    evidence.candidate_doctor.write_text(
        json.dumps({"schema": "capsem.installed_doctor.v1", "passed": False})
    )
    with pytest.raises(SystemExit, match="probe failed"):
        module.exact_installed_transition_rows(pairing, transport, evidence)


def _first_release_manifests(tmp_path: Path, module):
    """A channel serving nothing, and the candidate that would be its first release."""
    after_path = tmp_path / "after" / "Capsem_9.9.9_amd64.deb"
    after_path.parent.mkdir(parents=True)
    after_path.write_bytes(b"exact first release package")
    after_artifact = module.ArtifactIdentity.from_path(
        after_path,
        version="9.9.9",
        platform="linux",
        architecture="amd64",
    )
    after_manifest = _manifest(after_artifact)
    after_manifest["channel"] = "stable"
    after_manifest["profiles"] = {"code": {"revision": "code-1"}, "co-work": {"revision": "cw-1"}}
    # What `project-first-channel-before.py` writes for a channel whose published
    # graph was retired: an authority that offers nothing at all.
    before_manifest = {"channel": "stable", "packages": [], "profiles": {}}
    return before_manifest, after_manifest, after_artifact


def test_a_channel_serving_nothing_classifies_as_a_first_release(tmp_path: Path) -> None:
    """The case that blocked this line: retired predecessor, so no upgrade to prove."""
    module = _load_module()
    classifier = _load_first_release()
    before_manifest, after_manifest, after_artifact = _first_release_manifests(tmp_path, module)

    kind, profiles = classifier.classify_pairing_inputs(
        channel="stable",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=None,
        after_artifact=after_artifact,
    )

    assert kind is classifier.TransitionKind.FRESH_INSTALL
    # Every declared profile is staged: none of them was ever served.
    assert profiles == ("co-work", "code")


def test_a_first_release_pairing_carries_no_predecessor_identity(tmp_path: Path) -> None:
    module = _load_module()
    classifier = _load_first_release()
    before_manifest, after_manifest, after_artifact = _first_release_manifests(tmp_path, module)

    before, after = classifier.validate_pairing_inputs(
        kind=classifier.TransitionKind.FRESH_INSTALL,
        channel="stable",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=None,
        after_artifact=after_artifact,
        changed_profiles=("co-work", "code"),
    )

    assert before is None
    assert after.package_version == "9.9.9"


def test_the_caller_and_the_validator_agree_on_a_first_release(tmp_path: Path) -> None:
    """The rule was written twice and the two disagreed on a first release.

    `validate_pairing_inputs` demands a non-empty changed-profile set for every
    kind but `BINARY_ONLY`. Its caller in `local-release-glowup.py` listed only
    the two profile transitions and passed an empty set otherwise -- so
    `FRESH_INSTALL` raised "fresh_install release pairing requires changed
    profiles" whatever it was handed, and that is the pairing a first release
    makes.

    Both halves had tests of their own. What had none was their composition, so
    this feeds the caller's decision to the validator rather than restating
    either answer, and checks the caller asks instead of deciding.
    """
    module = _load_module()
    classifier = _load_first_release()

    assert "requires_changed_profiles(transition)" in LOCAL_GLOWUP_PATH.read_text(
        encoding="utf-8"
    ), "the glow-up decides for itself which pairings name the profiles they stage"
    for kind in classifier.TransitionKind:
        assert module.requires_changed_profiles(kind) is (
            kind is not module.TransitionKind.BINARY_ONLY
        ), kind

    before_manifest, after_manifest, after_artifact = _first_release_manifests(tmp_path, module)
    before_bytes = json.dumps(before_manifest).encode()
    after_bytes = json.dumps(after_manifest).encode()
    kind, profiles = classifier.classify_pairing_inputs(
        channel="stable",
        before_manifest_bytes=before_bytes,
        after_manifest_bytes=after_bytes,
        before_artifact=None,
        after_artifact=after_artifact,
    )

    # Exactly what the glow-up now does with the answer it was given.
    selected = profiles if module.requires_changed_profiles(kind) else ()
    before, after = classifier.validate_pairing_inputs(
        kind=kind,
        channel="stable",
        before_manifest_bytes=before_bytes,
        after_manifest_bytes=after_bytes,
        before_artifact=None,
        after_artifact=after_artifact,
        changed_profiles=selected,
    )

    assert kind is classifier.TransitionKind.FRESH_INSTALL
    assert before is None and after.package_version == "9.9.9"


def test_a_published_channel_still_requires_its_predecessor_package(tmp_path: Path) -> None:
    """The absence must not be able to downgrade a real upgrade into a fresh install."""
    module = _load_module()
    classifier = _load_first_release()
    _, after_manifest, after_artifact = _first_release_manifests(tmp_path, module)
    published_before = json.loads(json.dumps(after_manifest))
    published_before["profiles"] = {"code": {"revision": "code-0"}}

    with pytest.raises(classifier.GlowupContractError, match="requires a public-before package"):
        classifier.validate_pairing_inputs(
            kind=classifier.TransitionKind.PROFILE_THEN_BINARY,
            channel="stable",
            before_manifest_bytes=json.dumps(published_before).encode(),
            after_manifest_bytes=json.dumps(after_manifest).encode(),
            before_artifact=None,
            after_artifact=after_artifact,
            changed_profiles=("co-work", "code"),
        )


def test_a_graph_with_profiles_but_no_package_is_not_a_first_release() -> None:
    """Half-empty is broken, not fresh -- calling it fresh would skip the upgrade proof."""
    classifier = _load_first_release()
    half_empty = {"channel": "stable", "packages": [], "profiles": {"code": {"revision": "code-0"}}}

    assert classifier.public_before_is_unpublished(json.dumps(half_empty).encode()) is False
    assert (
        classifier.public_before_is_unpublished(
            json.dumps({"channel": "stable", "packages": [], "profiles": {}}).encode()
        )
        is True
    )


def test_a_first_release_cannot_claim_a_predecessor_it_never_served(tmp_path: Path) -> None:
    classifier = _load_first_release()
    module = _load_module()
    before_manifest, _, _ = _first_release_manifests(tmp_path, module)
    stray = tmp_path / "stray_1.5.0_amd64.deb"
    stray.write_bytes(b"a package the channel never published")

    with pytest.raises(SystemExit, match="published none"):
        classifier.resolve_public_before_package(
            supplied=stray,
            before_manifest_bytes=json.dumps(before_manifest).encode(),
        )

    assert classifier.resolve_public_before_package(
        supplied=None,
        before_manifest_bytes=json.dumps(before_manifest).encode(),
    ) == (None, None)


def _unpublished_profile_inputs(root: Path) -> Path:
    """The cohort a first release actually gets: a manifest offering nothing.

    This is what `project-first-channel-before.py` writes and what
    `fetch-release-inputs` verifies with `allow-empty-*`, so the report carries
    no artifact rows and there is nothing on disk to digest.
    """
    root.mkdir(parents=True, exist_ok=True)
    manifest = {"channel": "stable", "packages": [], "profiles": {}}
    (root / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
    (root / "release-inputs.json").write_text(
        json.dumps(
            {
                "schema": "capsem.release_inputs.v1",
                "kind": "profiles",
                "manifest_url": "file:///public-before/manifest.json",
                "allow_empty_profiles": True,
                "artifacts": [],
            }
        ),
        encoding="utf-8",
    )
    return root


def test_a_first_release_stages_a_transport_with_no_predecessor_package(tmp_path: Path) -> None:
    """The public-before side of a first release has a manifest and nothing else."""
    module = _load_local_glowup()
    inputs = _unpublished_profile_inputs(tmp_path / "before-inputs")

    staged_manifest, staged_package = module._stage_exact_transport_release(
        label="before",
        manifest_path=inputs / "manifest.json",
        package_path=None,
        profile_inputs=inputs,
        dist=tmp_path / "dist",
        base_url="http://127.0.0.1:9/base",
    )

    assert staged_package is None
    # The site still has to answer with the empty graph before the candidate is
    # promoted, so the manifest is projected even though nothing is staged.
    assert json.loads(staged_manifest.read_text()) == {
        "channel": "stable",
        "packages": [],
        "profiles": {},
    }
    assert not (tmp_path / "dist" / "transitions" / "before" / "package").exists()


def test_installed_status_is_read_from_the_installed_home() -> None:
    """`capsem status` must be told which home to look at.

    The verifier takes `--capsem-home` and uses it for file checks, but ran
    `capsem status` with no environment at all -- so `capsem` read whatever
    `CAPSEM_HOME` and `CAPSEM_RUN_DIR` the caller happened to export. Under the
    release pairing gate that is the gate's own isolated test home, which has
    no service in it, so a correctly installed and running product reported:

        installed release verification failed: capsem status is missing
        'Running:   true'

    The glow-up script's own readiness loop passed moments earlier precisely
    because it sets both variables before calling the same binary.
    """
    source = (PROJECT_ROOT / "scripts" / "verify-installed-release.py").read_text(encoding="utf-8")
    start = source.index('[str(args.capsem), "status"]')
    # To the end of the `subprocess.run(...)` call: a naive cut at the first
    # `)` lands inside `str(args.capsem)` and reads none of the arguments.
    invocation = source[start : source.index("\n    )", start)]
    assert "env=" in invocation, (
        "`capsem status` is run without an environment, so it reports on "
        "whichever CAPSEM_HOME the caller exported rather than on the "
        "installation being verified"
    )


def test_transition_verdict_uses_the_structured_product_audit() -> None:
    shell = _shell_of_local_glowup()
    body = embedded_shell.function_bodies(shell)["observe_update_transition"]

    assert '"$CAPSEM_HOME_DIR/logs/update.log"' in body
    assert "release_transition.py" in body
    assert "candidate-manifest-sha256" in body
    assert "service_log_grep" not in body
    assert "journalctl" not in body


def test_a_failed_rejection_wait_says_why() -> None:
    """Every ruled-out cause is printed, not left to be guessed at.

    Diagnosing the last failure meant reading the service source to learn
    where it logs, checking the unit for `--parent-pid`, and finding the poll
    interval -- none of which the failure output contained.
    """
    source = INSTALLED_PROBE_PATH.read_text(encoding="utf-8")
    dump = source[source.index("dump_update_diagnostics() {{") :]
    dump = dump[: dump.index("\n}}")]
    for evidence in (
        "automatic release",  # did the polling loop start, and decide what
        "service_log",  # the service's own tracing
        "update.log",  # what the updater recorded
        "systemctl --user status",  # does systemd think the unit is up
        "show-environment",  # was the poll interval override applied
        "journalctl",  # systemd's own view, for contrast
    ):
        assert evidence in dump, f"the diagnostic dump never shows {evidence}"
    assert "--since" not in dump, (
        "the audit marker is a line number, not a journal timestamp; passing it "
        "to journalctl --since hides the systemd evidence behind a parse error"
    )


def test_the_service_log_is_matched_by_pattern_not_by_a_fixed_name() -> None:
    """`service.log` is a rotation pattern, not a file.

    `telemetry::init` hands `run_dir/service.log` to a daily rolling appender,
    which writes `service.<date>.log`. Waiting on the literal name polled a
    path that never exists -- reported by the diagnostics as "No such file or
    directory" after three minutes of waiting.
    """
    source = INSTALLED_PROBE_PATH.read_text(encoding="utf-8")
    listing = source[source.index("service_logs() {{") :]
    listing = listing[: listing.index("\n}}")]
    assert "service*.log" in listing, (
        "the glow-up looks for a fixed log name; rotation means it must glob:\n" + listing
    )


# ---------------------------------------------------------------------------
# A proof that passes for the wrong reason.
#
# `wait_for_automatic_rejection` waited for `automatic release update failed`
# and nothing more. That message is what the service logs for *any* failed
# update cycle, so the tamper proof passed on run 32605810523 while the
# service was failing for an entirely unrelated reason -- a manifest URL the
# product would not accept, logged as
#
#     automatic release update failed ... release channel check failed
#     ... manifest_url must be an http(s) channel manifest URL
#
# The tampered manifest was never reached, let alone rejected. A green tamper
# proof over an update that never ran is worse than no proof: it is a report
# that the security property holds, issued by a check that did not test it.
# ---------------------------------------------------------------------------


def test_the_tamper_wait_requires_a_tamper_specific_message() -> None:
    module = _load_transition_module()
    source = "https://release.test/assets/nightly/manifest.json"
    candidate = "3" * 64
    previous = "2" * 64
    rows = [
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_fetched",
            "source": source,
            "candidate_manifest_sha256": candidate,
        },
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_rejected",
            "source": source,
            "candidate_manifest_sha256": candidate,
            "previous": {"manifest_sha256": previous},
            "current": {"manifest_sha256": previous},
            "error": "network unavailable",
        },
    ]

    with pytest.raises(module.TransitionEvidenceError, match="exact rejection cause"):
        module.build_transition_verdict(
            rows,
            kind="tampered_artifact",
            result="rejected",
            source=source,
            candidate_manifest_sha256=candidate,
            previous_manifest_sha256=previous,
        )


def test_the_tamper_wait_accepts_the_products_integrity_error() -> None:
    module = _load_transition_module()
    source = "https://release.test/assets/nightly/manifest.json"
    candidate = "3" * 64
    previous = "2" * 64
    rows = [
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_fetched",
            "source": source,
            "candidate_manifest_sha256": candidate,
        },
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_rejected",
            "source": source,
            "candidate_manifest_sha256": candidate,
            "previous": {"manifest_sha256": previous},
            "current": {"manifest_sha256": previous},
            "error": (
                "stage verified update candidate: profile config "
                "https://release.test/profiles/code/profile.toml "
                "failed size or digest verification"
            ),
        },
    ]

    verdict = module.build_transition_verdict(
        rows,
        kind="tampered_artifact",
        result="rejected",
        source=source,
        candidate_manifest_sha256=candidate,
        previous_manifest_sha256=previous,
    )

    assert verdict["preserved_previous"] is True


def test_neither_rejection_wait_can_be_satisfied_by_the_other() -> None:
    module = _load_transition_module()
    source = "https://release.test/assets/nightly/manifest.json"
    candidate = "4" * 64
    previous = "2" * 64
    rows = [
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_fetched",
            "source": source,
            "candidate_manifest_sha256": candidate,
        },
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_rejected",
            "source": source,
            "candidate_manifest_sha256": candidate,
            "previous": {"manifest_sha256": previous},
            "current": {"manifest_sha256": previous},
            "error": "profile requires Capsem 9999.0.0 or newer",
        },
    ]

    verdict = module.build_transition_verdict(
        rows,
        kind="incompatible_profile",
        result="rejected",
        source=source,
        candidate_manifest_sha256=candidate,
        previous_manifest_sha256=previous,
    )
    assert verdict["preserved_previous"] is True
    with pytest.raises(module.TransitionEvidenceError, match="exact rejection cause"):
        module.build_transition_verdict(
            rows,
            kind="tampered_artifact",
            result="rejected",
            source=source,
            candidate_manifest_sha256=candidate,
            previous_manifest_sha256=previous,
        )


def test_transition_verdict_requires_fetch_and_terminal_for_the_same_digest() -> None:
    module = _load_transition_module()
    source = "https://release.test/assets/nightly/manifest.json"
    candidate = "5" * 64
    rows = [
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_fetched",
            "source": source,
            "candidate_manifest_sha256": candidate,
        },
        {
            "schema": module.UPDATE_AUDIT_SCHEMA,
            "event": "release_candidate_activated",
            "source": source,
            "candidate_manifest_sha256": "6" * 64,
            "current": {"manifest_sha256": candidate},
        },
    ]

    with pytest.raises(module.TransitionEvidenceError, match="no exact-candidate activated"):
        module.build_transition_verdict(
            rows,
            kind="binary_only",
            result="activated",
            source=source,
            candidate_manifest_sha256=candidate,
        )


def test_update_audit_marker_excludes_an_old_matching_transition(tmp_path: Path) -> None:
    module = _load_transition_module()
    source = "https://release.test/assets/nightly/manifest.json"
    candidate = "7" * 64
    audit = tmp_path / "update.log"
    audit.write_text(
        "\n".join(
            json.dumps(row)
            for row in (
                {
                    "schema": module.UPDATE_AUDIT_SCHEMA,
                    "event": "release_candidate_fetched",
                    "source": source,
                    "candidate_manifest_sha256": candidate,
                },
                {
                    "schema": module.UPDATE_AUDIT_SCHEMA,
                    "event": "release_candidate_activated",
                    "source": source,
                    "candidate_manifest_sha256": candidate,
                    "current": {"manifest_sha256": candidate},
                },
            )
        )
        + "\n"
    )

    assert module.load_update_audit(audit, after_line=2) == []
    with pytest.raises(module.TransitionEvidenceError, match=r"does not prove.*fetched"):
        module.build_transition_verdict(
            module.load_update_audit(audit, after_line=2),
            kind="binary_only",
            result="activated",
            source=source,
            candidate_manifest_sha256=candidate,
        )


def test_the_polling_url_is_shaped_like_a_channel_manifest() -> None:
    """The product will not fetch a manifest from any other path.

    `channel_manifest_url` requires `<base>/assets/<channel>/manifest.json`:
    an `assets` segment, a channel after it, ending in the manifest. That is
    deliberate and is what the real release site serves.

    The glow-up served `/transitions/current/manifest.json`, so every
    automatic-update cycle in the proof died before fetching anything, with
    "release channel check failed". The tamper proof passed anyway -- it
    accepted any failed cycle -- and the incompatible-profile proof, which
    names its own cause, timed out. Neither had exercised an update.
    """
    source = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text(encoding="utf-8")
    routes = re.findall(r'current_route = f?"([^"]+)"', source)
    assert routes, "no polling route found; this guard is reading the wrong thing"

    unshaped = [route for route in routes if "/assets/" not in route]
    assert not unshaped, (
        "these polling routes have no `assets` segment, so `channel_manifest_url` "
        "rejects them before a single byte is fetched:\n  " + "\n  ".join(unshaped)
    )


# ---------------------------------------------------------------------------
# Does a promoted manifest actually reach the service?
#
# Attempt 37 finally got the update loop running -- and then accepted the
# tampered manifest. The update audit says why:
#
#     previous.manifest_sha256: 496137015cc134453f...
#     current.manifest_sha256:  496137015cc134453f...   (identical)
#     changed_fields: ["source", "origin"]
#
# Only the URL changed. The service fetched the *before* manifest for three
# minutes, so there was nothing to reject and the proof timed out having
# proved nothing -- the same vacuity as before, one layer deeper.
#
# Promotion is an `os.replace` into a directory a `SimpleHTTPRequestHandler`
# serves, and that handler answers conditional requests with 304. This asks
# the question directly, in a second, instead of over a two-hour release lane.
# ---------------------------------------------------------------------------


def test_a_promoted_manifest_is_what_the_server_serves(tmp_path: Path) -> None:
    """The one step between staging a tamper and the service seeing it."""
    glowup = _load_local_glowup()
    served = tmp_path / "dist" / "transitions" / "assets" / "stable"
    served.mkdir(parents=True)
    current = served / "manifest.json"
    current.write_text('{"schema":"before"}', encoding="utf-8")

    tampered = tmp_path / "tampered.json"
    tampered.write_text('{"schema":"tampered"}', encoding="utf-8")

    with glowup.local_release_server(tmp_path / "dist") as base_url:
        url = f"{base_url}/transitions/assets/stable/manifest.json"
        first = urllib.request.urlopen(url, timeout=10).read().decode()
        assert json.loads(first)["schema"] == "before"

        glowup.promote_exact_manifest(tampered, current)

        second = urllib.request.urlopen(url, timeout=10).read().decode()
        assert json.loads(second)["schema"] == "tampered", (
            "the promoted manifest is not what the server hands out, so a "
            "tamper proof waits on bytes the service will never see"
        )


def test_a_promotion_within_one_second_is_still_served(tmp_path: Path) -> None:
    """`Last-Modified` has one-second resolution.

    The glow-up stages and promotes in quick succession, and an HTTP cache
    keyed on a timestamp that has not ticked cannot tell the two apart.
    """
    glowup = _load_local_glowup()
    served = tmp_path / "dist"
    served.mkdir()
    current = served / "manifest.json"
    current.write_text('{"schema":"before"}', encoding="utf-8")
    tampered = tmp_path / "tampered.json"
    tampered.write_text('{"schema":"tampered"}', encoding="utf-8")

    with glowup.local_release_server(served) as base_url:
        url = f"{base_url}/manifest.json"
        request = urllib.request.Request(url)
        with urllib.request.urlopen(request, timeout=10) as response:
            last_modified = response.headers.get("Last-Modified")
        glowup.promote_exact_manifest(tampered, current)

        conditional = urllib.request.Request(url)
        if last_modified:
            conditional.add_header("If-Modified-Since", last_modified)
        try:
            body = urllib.request.urlopen(conditional, timeout=10).read().decode()
        except urllib.error.HTTPError as error:  # 304 has no body
            raise AssertionError(
                "the server answered a conditional request with "
                f"{error.code}; a client that caches never sees the promotion"
            ) from error
        assert json.loads(body)["schema"] == "tampered", body


def test_the_tamper_proof_checks_the_wire_before_it_waits() -> None:
    """Prove the tamper is being served, then wait for the reaction.

    Attempt 37 waited three minutes and reported "automatic rejection did not
    happen". The service had in fact seen no new manifest at all -- zero
    update-audit entries in those three minutes -- so the question was never
    whether it rejected the tamper, but whether the tamper reached it. The
    proof could not tell those apart, and neither could I without a CI cycle.

    A staging failure and a security failure are different failures. One of
    them means the product is broken.
    """
    staging = _shell_of_local_glowup()
    checks = staging.count("assert_manifest_served ")
    assert checks >= 2, (
        "nothing proves the staged manifest is what the URL returns, so a "
        f"timeout cannot distinguish 'did not reject' from 'never saw it' "
        f"({checks} call sites; both rejection proofs need one)"
    )


def test_the_wire_check_names_which_kind_of_failure_it_found() -> None:
    """A staging failure and a security failure are different failures.

    The message has to say which, because the next reader will be looking at
    a three-minute timeout with no other clue -- which is the situation this
    whole sequence of fixes keeps arriving at.
    """
    staging = _shell_of_local_glowup()
    body = staging[staging.index("assert_manifest_served() {") :]
    body = body[: body.index("\n}")]
    assert "staging failure" in body, body


def test_linux_glowup_proves_background_asset_hydration() -> None:
    """Linux qualification must prove that install hydration stays asynchronous."""
    staging = _shell_of_local_glowup()
    fresh_install = staging[staging.index("sudo apt-get remove --purge -y capsem") :]
    fresh_install = fresh_install[: fresh_install.index(" update --yes --channel nightly")]

    assert 'grep -Fq "event=manifest_installed"' in fresh_install
    assert 'if grep -Fq "event=assets_hydrated"' in fresh_install
    assert "package installer synchronously hydrated VM assets" in fresh_install
    assert 'assets status --profile "$profile" --json' in staging
    assert 'status.get("ready") and not status.get("downloading")' in staging
    assert (
        'wait_for_profile_assets code "$EVIDENCE_DIR/code-assets-after-install.json"'
        in fresh_install
    )
    assert (
        'wait_for_profile_assets co-work "$EVIDENCE_DIR/co-work-assets-after-install.json"'
        in fresh_install
    )
    assert fresh_install.index("event=manifest_installed") < fresh_install.index(
        "wait_for_profile_assets code"
    )
    assert fresh_install.index("wait_for_profile_assets co-work") < fresh_install.index(
        "probe_installed_transition fresh-stable"
    )


def _shell_of_local_glowup() -> str:
    return "\n".join(
        (
            embedded_shell.shell_of(LOCAL_GLOWUP_PATH),
            embedded_shell.shell_of(INSTALLED_PROBE_PATH),
        )
    )
