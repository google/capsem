"""Source doubles only: the parent executes the coherent package matrix."""

from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate.docker import Docker
from capsem_builder.release.tools import deb_preunpack_probe as probe
from capsem_builder.release.tools import preinstall_resource as resource
from capsem_builder.release.tools.finalize_binary_staging_fixtures import _write_synthetic_deb

ROOT = Path(__file__).resolve().parents[3]


@pytest.fixture(autouse=True)
def no_real_docker_cleanup(monkeypatch: pytest.MonkeyPatch) -> None:
    # Lifetime behavior has separate recorded-owner/real-signal tests.
    monkeypatch.setattr(Docker, "remove", lambda *args, **kwargs: None)


@pytest.fixture
def package(tmp_path: Path) -> Path:
    tree = tmp_path / "tree"
    (tree / "DEBIAN").mkdir(parents=True)
    (tree / "DEBIAN/control").write_text(
        "Package: capsem\nArchitecture: arm64\nDepends: libc6 (>= 2.39)\n"
        "Pre-Depends: libc6 (>= 2.39), libgcc-s1 (>= 4.2)\n"
    )
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    header[16:20] = b"\x03\x00\xb7\x00"
    binary = tree / "usr/bin/capsem-ledger"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(bytes(header) + b"non-executable source fixture")
    binary.chmod(0o755)
    (tree / "DEBIAN/preinst").write_bytes(probe.canonical_preinst(ROOT))
    resource.embed(tree, repository=ROOT, limits=resource.load_limits(ROOT))
    output = tmp_path / "candidate.deb"
    _write_synthetic_deb(tree, output)
    return output


def test_exact_candidate_resource_and_original_body_are_required(package: Path) -> None:
    assert probe.validate_package(package, repository=ROOT) == "arm64"


@pytest.mark.parametrize("member", ["usr/bin/capsem-ledger", "DEBIAN/preinst", "DEBIAN/control"])
def test_missing_resource_is_refused_before_docker(package: Path, member: str) -> None:
    tree = package.parent / "tree"
    (tree / member).unlink()
    package.unlink()
    _write_synthetic_deb(tree, package)
    with pytest.raises((ValueError, SystemExit), match=r"requires|missing|declare"):
        probe.validate_package(package, repository=ROOT)


@pytest.mark.parametrize("change", ["payload", "body", "predepends", "duplicate", "architecture"])
def test_changed_resource_body_or_absent_predepends_refused(package: Path, change: str) -> None:
    tree = package.parent / "tree"
    if change == "payload":
        with (tree / "usr/bin/capsem-ledger").open("ab") as target:
            target.write(b"changed")
    elif change == "body":
        with (tree / "DEBIAN/preinst").open("ab") as target:
            target.write(b"\nexit 0\n")
    elif change == "predepends":
        path = tree / "DEBIAN/control"
        path.write_text("Package: capsem\nArchitecture: arm64\nDepends: libc6 (>= 2.39)\n")
    elif change == "duplicate":
        with (tree / "DEBIAN/control").open("a") as target:
            target.write("Pre-Depends: bash\n")
    else:
        path = tree / "DEBIAN/control"
        path.write_text(path.read_text().replace("arm64", "amd64"))
    package.unlink()
    _write_synthetic_deb(tree, package)
    with pytest.raises((ValueError, SystemExit), match=r"identity|body|Pre-Depends|ELF"):
        probe.validate_package(package, repository=ROOT)


def test_resource_refusal_precedes_any_matrix_child(package: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    from capsem_builder.release.tools import prove_deb_platform_support as platform

    tree = package.parent / "tree"
    (tree / "DEBIAN/preinst").write_bytes(b"#!/bin/bash\nexit 0\n")
    package.unlink()
    _write_synthetic_deb(tree, package)

    def forbidden(*args, **kwargs):
        pytest.fail("resource refusal must precede Docker or other child preparation")

    monkeypatch.setattr(platform.subprocess, "run", forbidden)
    with pytest.raises(SystemExit, match="resource refused"):
        platform.main(["--package", str(package), "--config", str(ROOT / "config/gate.toml")])


@pytest.mark.parametrize("corrupt", [False, True])
def test_exact_validation_stage_is_removed_on_success_and_refusal(
    package: Path, monkeypatch: pytest.MonkeyPatch, corrupt: bool,
) -> None:
    stages = []
    original = probe.tempfile.TemporaryDirectory

    def tracked_stage(*args, **kwargs):
        stage = original(*args, **kwargs)
        stages.append(Path(stage.name))
        return stage

    monkeypatch.setattr(probe.tempfile, "TemporaryDirectory", tracked_stage)
    if corrupt:
        tree = package.parent / "tree"
        with (tree / "usr/bin/capsem-ledger").open("ab") as payload:
            payload.write(b"not the embedded bytes")
        package.unlink()
        _write_synthetic_deb(tree, package)
        with pytest.raises(ValueError, match="identity"):
            probe.validate_package(package, repository=ROOT)
    else:
        probe.validate_package(package, repository=ROOT)
    assert len(stages) == 1
    assert not stages[0].exists()


@pytest.mark.parametrize("supported", [True, False])
def test_original_package_single_disposable_owner_and_strict_receipt(
    package: Path, monkeypatch: pytest.MonkeyPatch, supported: bool,
) -> None:
    calls = []

    def run(argv, **kwargs):
        kwargs["original_bytes"] = kwargs["stdin"].read()
        calls.append((argv, kwargs))
        return SimpleNamespace(returncode=0, stdout=probe.receipt(supported), stderr=b"")

    monkeypatch.setattr(probe.subprocess, "run", run)
    assert probe.check_preunpack("fixture@sha256:known", package, "arm64", supported) == []
    argv, kwargs = calls.pop()
    assert "--network=none" in argv and "--pull=never" in argv
    assert argv[argv.index("--platform") + 1] == "linux/arm64"
    assert "--mount" not in argv and "-v" not in argv and "--rm" not in argv
    assert "--name" in argv
    assert kwargs["original_bytes"] == package.read_bytes()
    script = argv[argv.index("-c") + 1]
    assert "dpkg --unpack /probe/candidate.deb" in script
    assert "--configure" not in script and "apt" not in script
    assert "useradd" in script and "SUDO_USER=capsem-platform-probe" in script
    assert "pre-dependency problem" in script
    assert "dpkg-query --show" in script and "= unpacked" in script
    assert "capsem-preinstall.*" in script


@pytest.mark.parametrize("code,output", [(1, b""), (0, b""), (0, b"CAPSEM_PLATFORM_PREUNPACK_REFUSED\n")])
def test_failed_or_wrong_receipt_cannot_certify_supported(
    package: Path, monkeypatch: pytest.MonkeyPatch, code: int, output: bytes,
) -> None:
    monkeypatch.setattr(probe.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(
        returncode=code, stdout=output, stderr=b"tool or candidate failure",
    ))
    assert probe.check_preunpack("fixture", package, "arm64", True)


@pytest.mark.parametrize("overrides", [
    ["foreign-source", "foreign-image"],
    ["foreign-source", "foreign-image", "--probe-image", "foreign-source", "other-image"],
])
def test_platform_probe_refuses_unbound_or_duplicate_prepared_images(package, overrides, monkeypatch):
    from capsem_builder.release.tools import prove_deb_platform_support as platform

    def forbidden(*_args, **_kwargs):
        pytest.fail("an unbound prepared image must not reach a container")

    monkeypatch.setattr(platform, "check", forbidden)
    with pytest.raises(SystemExit, match="override"):
        platform.main(["--package", str(package), "--config", str(ROOT / "config/gate.toml"),
                       "--probe-image", *overrides])
