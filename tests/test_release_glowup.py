from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = PROJECT_ROOT / "scripts" / "release_glowup.py"
LOCAL_GLOWUP_PATH = PROJECT_ROOT / "scripts" / "local-release-glowup.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("release_glowup", MODULE_PATH)
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

    kind, profile = module.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is module.TransitionKind.BINARY_ONLY
    assert profile == ()

    after_manifest["profiles"]["experimental"]["revision"] = "experimental-2"
    kind, profile = module.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is module.TransitionKind.PROFILE_THEN_BINARY
    assert profile == ("experimental",)

    after_manifest["profiles"]["code"]["revision"] = "code-2"
    kind, profiles = module.classify_pairing_inputs(
        channel="nightly",
        before_manifest_bytes=json.dumps(before_manifest).encode(),
        after_manifest_bytes=json.dumps(after_manifest).encode(),
        before_artifact=before_artifact,
        after_artifact=after_artifact,
    )
    assert kind is module.TransitionKind.PROFILE_THEN_BINARY
    assert profiles == ("code", "experimental")


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


def test_local_channel_import_uses_the_typed_selected_revision_policy(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Public legacy revisions are imported, never accepted for new authoring."""
    module = _load_local_glowup()
    commands: list[list[str]] = []
    monkeypatch.setattr(module, "run", lambda command, **_kwargs: commands.append(command))

    module.build_channel(
        tmp_path / "capsem-admin",
        tmp_path / "manifest.json",
        tmp_path / "assets",
        tmp_path / "profiles",
        "stable",
        tmp_path / "dist",
        "http://127.0.0.1:31415",
        profile_revision_policy=module.ProfileRevisionPolicy.SELECTED_INPUT,
    )

    assert commands and commands[0][-2:] == [
        "--profile-revision-policy",
        "selected-input",
    ]


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
        "http://127.0.0.1:8765/transitions/current/manifest.json"
    )
    channel_catalog = json.loads(transport.channel_catalog.read_text())
    selected = channel_catalog["channels"]["nightly"]["manifests"]
    assert len(selected) == 1
    assert selected[0]["status"] == "current"
    assert selected[0]["url"] == "/transitions/current/manifest.json"
    assert selected[0]["digest"]["sha256"] == hashlib.sha256(
        transport.before_manifest.read_bytes()
    ).hexdigest()

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
    assert promoted_record["digest"]["sha256"] == hashlib.sha256(
        transport.after_manifest.read_bytes()
    ).hexdigest()

    candidates = module.stage_adversarial_exact_candidates(
        pairing,
        transport,
        output_dir=tmp_path / "adversarial",
    )

    assert after_manifest.read_bytes() == after_authority_bytes
    assert transport.after_manifest.read_text() == json.dumps(
        projected, indent=2, sort_keys=True
    ) + "\n"
    tampered = json.loads(candidates.tampered_manifest.read_text())
    incompatible = json.loads(candidates.incompatible_manifest.read_text())
    assert (
        tampered["profiles"]["code"]["architectures"][0]["images"][0]["digest"][
            "sha256"
        ]
        == "0" * 64
    )
    assert (
        tampered["profiles"]["code"]["architectures"][0]["images"][0]["digest"][
            "blake3"
        ]
        == "0" * 64
    )
    assert incompatible["profiles"]["code"]["min_capsem_version"] == "9999.0.0"
    assert candidates.tampered_manifest.read_bytes() != transport.after_manifest.read_bytes()
    assert candidates.incompatible_manifest.read_bytes() != transport.after_manifest.read_bytes()


def test_exact_installed_glowup_uses_service_poll_and_probes_each_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
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
                                "url": "/transitions/current/manifest.json",
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
        before_manifest_url="http://127.0.0.1:8765/transitions/before/manifest.json",
        after_manifest_url="http://127.0.0.1:8765/transitions/after/manifest.json",
        current_manifest_url="http://127.0.0.1:8765/transitions/current/manifest.json",
        channel_catalog_url="http://127.0.0.1:8765/transitions/channels.json",
        before_package=before_package,
        after_package=after_package,
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
    assert "wait_for_exact_transition" in after_script
    assert "probe_installed_transition candidate-after" in after_script
    assert "wait_for_automatic_rejection" in tamper_script
    assert "automatic release update failed" in tamper_script
    assert "tampered-before-manifest.json" in tamper_script
    assert "tampered-rejection.json" in tamper_script
    assert "wait_for_incompatible_profile_rejection" in incompatible_script
    assert "requires Capsem 9999.0.0 or newer" in incompatible_script
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
                                "url": "/transitions/current/manifest.json",
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
        before_manifest_url="http://127.0.0.1:8765/transitions/before/manifest.json",
        after_manifest_url="http://127.0.0.1:8765/transitions/after/manifest.json",
        current_manifest_url="http://127.0.0.1:8765/transitions/current/manifest.json",
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

    assert len(calls) == 4
    fresh_script = calls[0][-1]
    assert "probe_installed_transition fresh-install" in fresh_script
    assert "probe_installed_transition candidate-after" not in fresh_script
    assert transport.before_manifest_url not in fresh_script
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
    for path, kind in (
        (evidence.tamper_rejection, "tampered_artifact"),
        (evidence.incompatible_rejection, "incompatible_profile"),
    ):
        path.write_text(
            json.dumps(
                {
                    "schema": "capsem.installed_rejection.v1",
                    "kind": kind,
                    "result": "rejected",
                    "preserved_previous": True,
                    "blocked_reason": (
                        "requires Capsem 9999.0.0 or newer"
                        if kind == "incompatible_profile"
                        else None
                    ),
                }
            )
        )

    rows = module.exact_installed_transition_rows(pairing, evidence)

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
    evidence = module.ExactInstalledGlowupEvidence(
        fresh_installed=tmp_path / "fresh-installed.json",
        fresh_doctor=tmp_path / "fresh-doctor.json",
        fresh_winterfell=tmp_path / "fresh-winterfell.json",
        candidate_installed=tmp_path / "candidate-installed.json",
        candidate_doctor=tmp_path / "candidate-doctor.json",
        candidate_winterfell=tmp_path / "candidate-winterfell.json",
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
        path.write_text(
            json.dumps({"schema": "capsem.installed_doctor.v1", "passed": True})
        )
    for path in (
        evidence.fresh_winterfell,
        evidence.candidate_winterfell,
        evidence.preserved_winterfell,
    ):
        path.write_text(
            json.dumps({"schema": "capsem.installed_winterfell.v1", "passed": True})
        )
    evidence.tamper_rejection.write_text(
        json.dumps(
            {
                "schema": "capsem.installed_rejection.v1",
                "kind": "tampered_artifact",
                "result": "rejected",
                "preserved_previous": True,
            }
        )
    )
    evidence.incompatible_rejection.write_text(
        json.dumps(
            {
                "schema": "capsem.installed_rejection.v1",
                "kind": "incompatible_profile",
                "result": "rejected",
                "blocked_reason": "requires Capsem 9999.0.0 or newer",
                "preserved_previous": True,
            }
        )
    )

    rows = module.exact_installed_transition_rows(pairing, evidence)

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
            {
                "schema": "capsem.installed_rejection.v1",
                "kind": "tampered_artifact",
                "result": "activated",
                "preserved_previous": False,
            }
        )
    )
    with pytest.raises(SystemExit, match="rejection evidence failed"):
        module.exact_installed_transition_rows(pairing, evidence)
    evidence.tamper_rejection.write_text(
        json.dumps(
            {
                "schema": "capsem.installed_rejection.v1",
                "kind": "tampered_artifact",
                "result": "rejected",
                "preserved_previous": True,
            }
        )
    )
    evidence.candidate_doctor.write_text(
        json.dumps({"schema": "capsem.installed_doctor.v1", "passed": False})
    )
    with pytest.raises(SystemExit, match="probe failed"):
        module.exact_installed_transition_rows(pairing, evidence)
