from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = PROJECT_ROOT / "scripts" / "release_glowup.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("release_glowup", MODULE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
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
