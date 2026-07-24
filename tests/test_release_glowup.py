from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
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

    with pytest.raises(module.GlowupContractError, match="package|architecture"):
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
        selected_profile="experimental",
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
            selected_profile="experimental",
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
            selected_profile="work",
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
