"""Hermetic materialization contract for guest Rust cross-build containers."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import imagebases, imagebuild, initrd
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.plan import Plan
from capsem_builder.image import guestbuilder
from capsem_builder.image.config import load_guest_config
from capsem_builder.image.docker import GUEST_BINARIES, container_compile_agent
from capsem_builder.image.guestbuilder import image_repository, image_tag
from capsem_builder.image.models import ArchConfig
from helpers.gate import RecordingRunner
from pydantic import ValidationError

PROJECT_ROOT = Path(__file__).resolve().parents[3]
BUILD = load_guest_config(PROJECT_ROOT / "config/docker/image").build


def _arch(name: str = "arm64") -> ArchConfig:
    return BUILD.architectures[name]


def _seed_identity(root: Path) -> None:
    for relative in (
        BUILD.guest_rust_builder.dockerfile,
        *BUILD.guest_rust_builder.identity_inputs,
    ):
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT_ROOT / relative, destination)


def test_every_guest_rust_builder_base_is_an_exact_platform_child_manifest() -> None:
    """One exact, distinct child manifest is declared per architecture.

    Which base a given *target* resolves to is a separate question answered by
    `guestbuilder.environment`: a foreign target is cross-compiled from the
    host's child, so two targets normally share one base at build time. This
    contract is about the declarations being exact and per-platform, not about
    how many of them a single host ends up using.
    """
    refs = {arch.rust_builder_base_image for arch in BUILD.architectures.values()}

    assert len(refs) == len(BUILD.architectures)
    for arch in BUILD.architectures.values():
        repository, digest = arch.rust_builder_base_image.rsplit("@sha256:", 1)
        assert repository == "docker.io/library/rust"
        assert len(digest) == 64
        assert digest == digest.lower()


def test_guest_rust_builder_materializes_the_checked_in_lock_before_runtime() -> None:
    source = (PROJECT_ROOT / BUILD.guest_rust_builder.dockerfile).read_text(encoding="utf-8")

    assert "rust:slim-bookworm" not in source
    assert "COPY Cargo.toml Cargo.lock rust-toolchain.toml" in source
    assert "cargo fetch --locked" in source
    assert "apt-get" not in source
    assert "ENV RUSTUP_AUTO_INSTALL=0" in source
    assert "rustup toolchain list" in source
    assert "rustup target list --installed" in source
    assert "RUN rm -rf /prefetch" in source


def test_a_cross_image_materializes_its_target_and_asserts_it_landed() -> None:
    """`rustup target add` is permitted, and only in the cross setup layer.

    This assertion used to be `"rustup target add" not in source`, because
    every image was the target's own platform child and already carried its
    native target -- so an install could only mean the base had drifted.

    A cross image is built FROM the host's child and must add the foreign
    target. That is materialization at image-build time on the same
    network-open edge `cargo fetch --locked` already uses, not a runtime
    download: the target is still asserted present afterwards, so the runtime's
    `--locked --offline --network none` build never reaches the rustup proxy.
    """
    source = (PROJECT_ROOT / BUILD.guest_rust_builder.dockerfile).read_text(encoding="utf-8")

    add = source.index("rustup target add")
    assert 'if [ "${CROSS}" = "1" ]' in source
    assert source.index('if [ "${CROSS}" = "1" ]') < add
    # Asserted after the install, unconditionally, for both shapes.
    assert add < source.index("rustup target list --installed")
    assert "ARG CROSS" in source
    assert "ARG CROSS_PACKAGES" in source
    assert "apk add --no-cache ${CROSS_PACKAGES}" in source


def test_cross_package_selection_changes_only_the_cross_helper_identity() -> None:
    cross_target = "arm64" if guestbuilder.host_architecture(BUILD) == "x86_64" else "x86_64"
    native_target = guestbuilder.host_architecture(BUILD)
    changed_settings = BUILD.guest_rust_builder.model_copy(
        update={"cross_packages": ("clang21=21.1.2-r3",)}
    )
    changed_build = BUILD.model_copy(update={"guest_rust_builder": changed_settings})

    assert image_tag(changed_build, cross_target, PROJECT_ROOT) != image_tag(
        BUILD, cross_target, PROJECT_ROOT
    )
    assert image_tag(changed_build, native_target, PROJECT_ROOT) == image_tag(
        BUILD, native_target, PROJECT_ROOT
    )


@pytest.mark.parametrize(
    "packages",
    ((), ("clang21",), ("clang21=21.1.2-r2 extra",)),
)
def test_cross_packages_must_be_nonempty_exact_specs(packages: tuple[str, ...]) -> None:
    data = BUILD.guest_rust_builder.model_dump()
    data["cross_packages"] = packages

    with pytest.raises(ValidationError):
        type(BUILD.guest_rust_builder).model_validate(data)


@pytest.mark.parametrize(
    "image",
    (
        "rust:slim-bookworm",
        "docker.io/library/rust@sha256:short",
        "docker.io/library/rust@sha256:" + "A" * 64,
    ),
)
def test_mutable_or_malformed_guest_rust_builder_base_is_rejected(image: str) -> None:
    data = _arch().model_dump()
    data["rust_builder_base_image"] = image

    with pytest.raises(ValidationError):
        ArchConfig.model_validate(data)


@pytest.mark.parametrize(
    "changed",
    (
        BUILD.guest_rust_builder.dockerfile,
        *BUILD.guest_rust_builder.identity_inputs,
    ),
)
def test_guest_rust_builder_tag_is_keyed_by_every_materialized_input(
    tmp_path: Path,
    changed: str,
) -> None:
    for relative in (
        BUILD.guest_rust_builder.dockerfile,
        *BUILD.guest_rust_builder.identity_inputs,
    ):
        path = tmp_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"first {relative}\n", encoding="utf-8")

    first = image_tag(BUILD, "arm64", tmp_path)
    (tmp_path / changed).write_text("changed\n", encoding="utf-8")

    assert image_tag(BUILD, "arm64", tmp_path) != first


def test_every_guest_rust_builder_generation_is_owned_by_cache_policy() -> None:
    policy = load_policy(PROJECT_ROOT)
    assert policy.control is not None

    for arch_name in BUILD.architectures:
        repository = image_repository(BUILD, arch_name)
        resource = policy.control.docker.images[repository]
        assert resource.repository == repository
        assert resource.keep_previous == 0


def test_cold_prefetch_pulls_exact_rust_base_then_builds_locked_helper() -> None:
    config = gate_config.load(PROJECT_ROOT)
    runner = RecordingRunner(PROJECT_ROOT, failures=("docker image inspect",))

    # Resolved rather than spelled: which base an architecture builds from
    # depends on the host, so a literal here would assert the CI runner's CPU.
    resolved = guestbuilder.environment(BUILD, "arm64")

    imagebases.prefetch(runner, config, names=("arm64",))
    builder = image_tag(BUILD, "arm64", PROJECT_ROOT)
    runner.fail_on(f"docker image inspect {builder}")
    imagebases.materialize_rust_builders(runner, config, names=("arm64",))

    pulled = f"docker pull --platform {resolved.docker_platform} {resolved.base_image}"
    built = runner.matching(r"docker build .*Dockerfile\.guest-rust-builder")
    assert len(built) == 1
    assert f"--platform {resolved.docker_platform}" in built[0]
    assert "--network default" in built[0]
    assert f"BASE={resolved.base_image}" in built[0]
    assert f"RUST_TARGET={BUILD.architectures['arm64'].rust_target}" in built[0]
    assert f"CROSS={'1' if resolved.cross else '0'}" in built[0]
    packages = " ".join(resolved.cross_packages)
    assert f"CROSS_PACKAGES={packages}" in built[0]
    assert any(
        note.startswith(f"sealed arm64 guest Rust builder identity: {builder} ->")
        for note in runner.notes
    )
    runner.assert_order(pulled, r"docker build .*Dockerfile\.guest-rust-builder")


def test_warm_prefetch_uses_the_input_keyed_helper_without_registry_egress() -> None:
    config = gate_config.load(PROJECT_ROOT)
    runner = RecordingRunner(PROJECT_ROOT)

    imagebases.prefetch(runner, config, names=("arm64",))
    imagebases.materialize_rust_builders(runner, config, names=("arm64",))

    assert not runner.matching(r"docker pull")
    assert not runner.matching(r"docker build")


def test_prefetch_pulls_rust_bases_only_for_helper_consumers() -> None:
    config = gate_config.load(PROJECT_ROOT)
    runner = RecordingRunner(PROJECT_ROOT, failures=("docker image inspect",))

    imagebases.prefetch(
        runner,
        config,
        names=("arm64", "x86_64"),
        rust_names=("arm64",),
    )

    arm = BUILD.architectures["arm64"]
    x86 = BUILD.architectures["x86_64"]
    # Both guest bases are still per-target and pulled at their own platform.
    assert runner.ran(rf"docker pull --platform {arm.docker_platform} {arm.base_image}")
    assert runner.ran(rf"docker pull --platform {x86.docker_platform} {x86.base_image}")

    # The Rust builder base is the host's, because a foreign target is crossed
    # rather than emulated. Only the requested consumer's helper base is
    # pulled, and no architecture outside `rust_names` drags one in.
    resolved = guestbuilder.environment(BUILD, "arm64")
    assert runner.ran(rf"docker pull --platform {resolved.docker_platform} {resolved.base_image}")
    unused = {arch.rust_builder_base_image for arch in BUILD.architectures.values()} - {
        resolved.base_image
    }
    for reference in unused:
        assert not runner.ran(rf"docker pull .*{reference}")


def test_materialization_fails_closed_without_its_exact_rust_base() -> None:
    config = gate_config.load(PROJECT_ROOT)
    base = guestbuilder.environment(BUILD, "arm64").base_image
    runner = RecordingRunner(PROJECT_ROOT, failures=(base,))

    with pytest.raises(GateError, match="Rust builder base is missing"):
        imagebases.materialize_rust_builders(runner, config, names=("arm64",))

    assert not runner.ran(r"docker build")


def test_linux_helpers_cover_every_requested_target(monkeypatch: pytest.MonkeyPatch) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem_builder.gate.host.machine", lambda: "x86_64")

    assert imagebases.required_rust_builder_names(config) == ("arm64", "x86_64")
    assert imagebases.required_rust_builder_names(config, ("x86_64",)) == ("x86_64",)


def test_macos_helpers_cover_every_requested_target(monkeypatch: pytest.MonkeyPatch) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem_builder.gate.host.machine", lambda: "arm64")

    assert imagebases.required_rust_builder_names(config) == ("arm64", "x86_64")
    assert imagebases.required_rust_builder_names(config, ("x86_64",)) == ("x86_64",)


def test_standalone_macos_initrd_build_materializes_its_guest_rust_builder(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    plan = Plan("standalone-initrd")
    monkeypatch.setattr("capsem_builder.gate.initrd.host.on_macos", lambda: True)

    initrd.pack(plan, config)

    assert "materialize exact guest base images" in "\n".join(
        plan.step_named("initrd.guest-base").render()
    )
    assert "prove Docker can execute" in "\n".join(
        plan.step_named("initrd.guest-execution").render()
    )
    assert "materialize locked guest Rust builders" in "\n".join(
        plan.step_named("initrd.guest-builder").render()
    )
    assert plan.after_of("initrd.guest-execution") == {"initrd.guest-base"}
    assert plan.after_of("initrd.guest-builder") == {"initrd.guest-execution"}
    assert plan.after_of("initrd.guest-agents") == {"initrd.guest-builder"}


def test_macos_check_assets_proves_execution_before_materializing_helper(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    plan = Plan("standalone-check-assets")
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem_builder.gate.imagebuild.missing", lambda *_args: ["initrd.img"])

    imagebuild.check_assets(plan, config)

    assert plan.after_of("assets.guest-execution") == {"assets.doctor"}
    assert plan.after_of("assets.guest-builders") == {"assets.guest-execution"}
    assert plan.after_of("assets.asset-tools") == {"assets.guest-builders"}
    assert plan.after_of("assets.recovery-dependencies") == {"assets.asset-tools"}
    assert plan.step_named("assets.guest-builders").carry_checks
    assert plan.step_named("assets.asset-tools").carry_checks
    assert plan.step_named("assets.recovery-dependencies").carry_checks
    previous = "assets.recovery-dependencies"
    for profile in imagebuild.profiles(config):
        label = f"assets.image.{profile}.all.{config.host_arch().name}"
        assert plan.after_of(label) == {previous}
        previous = label


@patch("capsem_builder.image.docker.run_cmd")
@patch("capsem_builder.image.docker.detect_runtime", return_value="docker")
def test_cross_build_fails_closed_when_the_materialized_helper_is_missing(
    _detect: MagicMock, run: MagicMock, tmp_path: Path
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    _seed_identity(repo)
    run.side_effect = subprocess.CalledProcessError(1, ["docker", "image", "inspect"])

    with pytest.raises(RuntimeError, match="locked guest Rust builder is missing"):
        container_compile_agent(BUILD, "arm64", repo, tmp_path / "output")

    assert run.call_count == 1
    inspected = run.call_args.args[0]
    assert inspected[:3] == ["docker", "image", "inspect"]
    assert inspected[3:5] == ["--format", "{{.Os}}/{{.Architecture}}"]
    assert "--platform" not in inspected
    assert image_tag(BUILD, "arm64", repo) == inspected[-1]
    assert "pull" not in inspected
    assert "build" not in inspected


@patch("capsem_builder.image.docker.run_cmd")
@patch("capsem_builder.image.docker.detect_runtime", return_value="docker")
def test_cross_build_refuses_a_materialized_helper_for_the_wrong_platform(
    _detect: MagicMock, run: MagicMock, tmp_path: Path
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    _seed_identity(repo)
    expected = guestbuilder.environment(BUILD, "arm64").docker_platform
    wrong = "linux/arm64" if expected != "linux/arm64" else "linux/amd64"
    run.return_value = MagicMock(stdout=f"{wrong}\n")

    with pytest.raises(RuntimeError, match=f"resolves to {wrong}, expected {expected}"):
        container_compile_agent(BUILD, "arm64", repo, tmp_path / "output")

    assert run.call_count == 1


@patch("capsem_builder.image.docker.run_cmd")
@patch("capsem_builder.image.docker.detect_runtime", return_value="docker")
def test_cross_build_runs_from_materialized_inputs_with_network_denied(
    _detect: MagicMock, run: MagicMock, tmp_path: Path
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    _seed_identity(repo)
    output = tmp_path / "output"

    def produce(cmd, **_kwargs):
        if "inspect" in cmd:
            return MagicMock(stdout=f"{guestbuilder.environment(BUILD, 'arm64').docker_platform}\n")
        if "run" in cmd:
            for binary in GUEST_BINARIES:
                (output / binary).write_bytes(b"elf")
        return MagicMock(stdout="")

    run.side_effect = produce

    container_compile_agent(BUILD, "arm64", repo, output)

    command = next(call.args[0] for call in run.call_args_list if "run" in call.args[0])
    script = command[-1]
    assert command[command.index("--network") + 1] == "none"
    assert image_tag(BUILD, "arm64", repo) in command
    assert "/usr/local/cargo/registry" not in command
    assert "/usr/local/cargo/git" not in command
    assert "/usr/local/rustup" not in command
    assert '[ "$b" != Cargo.lock ]' not in script
    assert "apt-get" not in script
    assert "rustup target add" not in script
    assert "cargo build --locked --offline" in script
    assert command[-3:-1] == ["sh", "-c"]
