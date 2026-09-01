"""One exact `.deb`, installed clean, and asked to prove it works.

The narrow assertion worth keeping in view is the version check on each binary.
A package can install cleanly while carrying binaries from an earlier build --
the package metadata and the ELF inside it are stamped separately -- so every
file-existence check in the world passes on that package, and it ships.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.content import ProfileContent
from capsem_builder.gate.debproof import DebProof
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.sourcecommit import SourceCommit
from helpers.gate import RecordingRunner
from profile_content import materialize_required_artifacts

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(PROJECT_ROOT)
PROOF = CONFIG.package.proof
VERSION = "9.9.9"
SOURCE_COMMIT = SourceCommit("0" * 40)
PACKAGE_ROOT = Path(CONFIG.outputs.packages)


@pytest.fixture(autouse=True)
def _qualified_install_image(monkeypatch: pytest.MonkeyPatch) -> None:
    """DebProof tests own the transaction, not install-image provenance."""
    monkeypatch.setattr(
        "capsem_builder.gate.installimage.require_local_image",
        lambda _runner, _config: "sha256:" + "0" * 64,
    )


def _checkout(tmp_path: Path) -> Path:
    (tmp_path / "config").mkdir(parents=True)
    (tmp_path / "config" / "gate.toml").write_text(
        (PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8")
    )
    (tmp_path / PACKAGE_ROOT).mkdir(parents=True)
    package = tmp_path / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb"
    package.write_text("package bytes")
    return tmp_path


def _content(root: Path) -> ProfileContent:
    config = gate_config.load(root)
    content = ProfileContent.standalone(config)
    payload = json.dumps(
        {
            "assets": {
                "current": "test",
                "releases": {
                    "test": {"arches": {name: {} for name in config.architectures}},
                },
            },
        }
    ).encode()
    content.assets.mkdir(parents=True)
    (content.assets / config.install.manifest_name).write_bytes(payload)
    materialize_required_artifacts(config, content.assets)
    config_manifest = content.config_manifest(config)
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(payload)
    profile = content.profiles(config) / "code/profile.toml"
    profile.parent.mkdir(parents=True)
    profile.write_text("name = 'code'\n")
    return content


def _replies(*, version: str = VERSION, status: str | None = None) -> dict[str, str]:
    ready = len(list((PROJECT_ROOT / "config" / "profiles").glob("*/profile.toml")))
    return {
        "dpkg-deb -f": version,
        "-f=${Status}": "install ok installed",
        "-f=${Version}": version,
        "--version": f"capsem {version}",
        "capsem status": status
        if status is not None
        else (
            "Installed: true\nRunning:   true\nService:   ok\nGateway:   ok\n"
            f"Profiles:  {ready}/{ready} ready\n"
        ),
        "systemctl is-system-running": "running",
    }


def _proof(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, **kwargs
) -> tuple[DebProof, RecordingRunner]:
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: True)
    root = _checkout(tmp_path)
    runner = RecordingRunner(root, replies=_replies(**kwargs.pop("replies", {})), **kwargs)
    built = DebProof(
        runner,
        package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
        content=_content(root),
        manifest_url="file:///src/m.json",
        channel="nightly",
        source_commit=SOURCE_COMMIT,
        sleep=lambda _seconds: None,
    )
    return built, runner


# ---------------------------------------------------------------------------
# Inputs
# ---------------------------------------------------------------------------


def test_only_a_package_this_checkout_built_is_accepted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Anything outside cache/target/packages/ has no package-build provenance."""
    root = _checkout(tmp_path)
    elsewhere = tmp_path / "elsewhere.deb"
    elsewhere.write_text("bytes")

    with pytest.raises(GateError, match="only accepts cache/target/packages/"):
        DebProof(
            RecordingRunner(root),
            package=elsewhere,
            content=_content(root),
            manifest_url="file:///src/m.json",
            channel="nightly",
            source_commit=SOURCE_COMMIT,
        )


def test_an_unknown_channel_is_refused(tmp_path: Path) -> None:
    root = _checkout(tmp_path)

    with pytest.raises(GateError, match="unsupported exact package proof channel"):
        DebProof(
            RecordingRunner(root),
            package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
            content=_content(root),
            manifest_url="file:///src/m.json",
            channel="prod",
            source_commit=SOURCE_COMMIT,
        )


def test_a_host_without_kvm_refuses_rather_than_proving_less(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, _ = _proof(tmp_path, monkeypatch)
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: False)

    with pytest.raises(GateError, match="/dev/kvm"):
        proof.run()


# ---------------------------------------------------------------------------
# The proof
# ---------------------------------------------------------------------------


def test_the_checkout_is_not_mounted_at_all(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A package that only works because it wrote back into /src is not a
    package that works."""
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    started = runner.matching(r"docker run -d")[0]
    # Was `-v {root}:/src:ro`. Read-only was never the protection it looked
    # like: the container and every concurrent host step still shared inodes
    # over virtiofs, which is what killed a release run. The image carries the
    # source now, so there is nothing to share.
    assert f"-v {proof.root}:" not in started, f"the checkout is mounted again: {started}"


def test_exact_package_graph_is_checked_and_handed_off_before_dpkg(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    transcript = "\n".join(runner.rendered)
    extract = transcript.index("dpkg-deb --extract")
    builds = runner.matching(r"assets channel build")
    assert len(builds) == 2
    first_build = transcript.index(builds[0])
    record = transcript.index("record-binary")
    second_build = transcript.index(builds[1])
    check = transcript.index("assets channel check")
    handoff = transcript.index("install-manifest-request.sh write")
    install = transcript.index("dpkg -i")
    assert extract < first_build < record < second_build < check < handoff < install
    authoritative = f"{CONFIG.install.layout.channel}/{CONFIG.install.graph_manifest}"
    record_command = runner.matching(r"assets channel record-binary")[0]
    assert f"--manifest-path {authoritative}" in record_command
    assert f"--source-commit {SOURCE_COMMIT}" in transcript
    assert "--profile-revision-policy selected-input" in transcript
    assert "--network none" in runner.matching(r"docker run -d")[0]


def test_read_only_content_is_staged_before_record_binary_mutates_the_generated_graph(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    started = runner.matching(r"docker run -d")[0]
    assert f":{CONFIG.install.proof_assets_mount}:ro" in started
    assert f":{CONFIG.install.proof_config_mount}:ro" in started
    record = runner.matching(r"assets channel record-binary")[0]
    authoritative = f"{CONFIG.install.layout.channel}/{CONFIG.install.graph_manifest}"
    assert f"--manifest-path {authoritative}" in record
    assert (
        f"--manifest-path {CONFIG.install.proof_assets_mount}/{CONFIG.install.manifest_name}"
        not in record
    )
    assert not runner.ran(r"docker exec(?: [^ ]+)* capsem-install-test(?: |$)")


def test_runtime_dependency_authority_is_verified_before_dpkg(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    runner.assert_order(r"install-deb-runtime-dependencies\.py .*--verify-only", r"dpkg -i")


def test_every_shipped_binary_must_report_the_package_version(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    for name in PROOF.binaries:
        assert runner.ran(rf"test -x /usr/bin/{name}$")
    for name in PROOF.versioned_binaries:
        assert runner.ran(rf"/usr/bin/{name} --version")


def test_the_bundle_without_a_version_flag_is_not_asked_for_one(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    for name in PROOF.binaries_without_version:
        assert runner.ran(rf"test -x /usr/bin/{name}$")
        assert not runner.ran(rf"/usr/bin/{name} --version")


def test_a_binary_carrying_an_older_build_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The package metadata and the ELF inside it are stamped separately, so
    this is the only check that catches a stale binary in a fresh package."""
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: True)
    root = _checkout(tmp_path)
    replies = _replies()
    replies["--version"] = "capsem 0.0.1"
    runner = RecordingRunner(root, replies=replies)

    proof = DebProof(
        runner,
        package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
        content=_content(root),
        manifest_url="file:///src/m.json",
        channel="nightly",
        source_commit=SOURCE_COMMIT,
        sleep=lambda _seconds: None,
    )

    with pytest.raises(GateError, match="does not carry the package version"):
        proof.run()


def test_an_installed_version_that_disagrees_with_the_package_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: True)
    root = _checkout(tmp_path)
    replies = _replies()
    replies["-f=${Version}"] = "0.0.1"
    runner = RecordingRunner(root, replies=replies)

    with pytest.raises(GateError, match=r"expected 9\.9\.9"):
        DebProof(
            runner,
            package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
            content=_content(root),
            manifest_url="file:///src/m.json",
            channel="nightly",
            source_commit=SOURCE_COMMIT,
            sleep=lambda _seconds: None,
        ).run()


@pytest.mark.parametrize("absent", CONFIG.package.proof.status_requires)
def test_a_status_line_that_is_missing_fails_the_proof(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, absent: str
) -> None:
    """A package that installs and then cannot start its own service passes
    every file-existence check there is."""
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: True)
    root = _checkout(tmp_path)
    full = _replies()["capsem status"]
    replies = _replies()
    replies["capsem status"] = full.replace(absent, "")
    runner = RecordingRunner(root, replies=replies)

    with pytest.raises(GateError, match="status is missing"):
        DebProof(
            runner,
            package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
            content=_content(root),
            manifest_url="file:///src/m.json",
            channel="nightly",
            source_commit=SOURCE_COMMIT,
            sleep=lambda _seconds: None,
        ).run()


@pytest.mark.parametrize("counts", ["0/0", "1/3"])
def test_profiles_must_all_be_ready(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, counts: str
) -> None:
    """Zero of zero is the interesting one: it reads as success to anything
    that only compares the two numbers."""
    monkeypatch.setattr("capsem_builder.gate.host.device_available", lambda _path: True)
    root = _checkout(tmp_path)
    replies = _replies()
    replies["capsem status"] = (
        "Installed: true\nRunning:   true\nService:   ok\nGateway:   ok\n"
        f"Profiles:  {counts} ready\n"
    )
    runner = RecordingRunner(root, replies=replies)

    with pytest.raises(GateError, match="profiles are not all ready"):
        DebProof(
            runner,
            package=root / PACKAGE_ROOT / f"Capsem_{VERSION}_arm64.deb",
            content=_content(root),
            manifest_url="file:///src/m.json",
            channel="nightly",
            source_commit=SOURCE_COMMIT,
            sleep=lambda _seconds: None,
        ).run()


def test_the_release_and_shell_proofs_run_as_the_unprivileged_user(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Running them as root proves an installation no user would have."""
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    for script in (PROOF.verify_script, PROOF.shell_proof_script):
        matched = runner.matching(script.replace(".", r"\."))
        assert matched
        assert f"-u {CONFIG.install.guest_user.name}" in matched[0]


def test_vm_devices_are_granted_and_probed_as_the_runtime_user(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch)

    proof.run()

    started = runner.matching(r"docker run -d")[0]
    assert "--group-add" not in started
    assert (
        f"bash {CONFIG.install.vm_device_setup_script} {CONFIG.install.guest_user.name} "
        f"{CONFIG.install.systemd_command} /dev/kvm /dev/vhost-vsock"
    ) in started
    user = CONFIG.install.guest_user.name
    for device in CONFIG.install.vm_devices:
        assert runner.ran(rf"docker exec -u {user} .*test -r {device} -a -w {device}")


def test_the_container_is_removed_even_when_the_proof_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    proof, runner = _proof(tmp_path, monkeypatch, failures=["prove-installed-shell"])

    with pytest.raises(GateError):
        proof.run()

    assert runner.last_index_of(rf"docker rm -f -v {PROOF.container}") > runner.index_of(r"dpkg -i")
