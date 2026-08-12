"""What the install container can prove on this host, and what it must refuse.

Two host facts decide the answer and both used to sit in the middle of a
270-line recipe: whether `/dev/kvm` and `/dev/vhost-vsock` are usable, and
whether Colima's Rosetta binfmt registration is present. The second is checked
twice because a privileged systemd container has removed it -- which breaks
every later x86 build on the machine, not just the run that caused it.
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner
from helpers.profile_content import materialize_required_artifacts

from capsem.gate import config as gate_config
from capsem.gate.content import ProfileContent, SelectedInstallContent
from capsem.gate.errors import GateError
from capsem.gate.installcontainer import InstallContainer

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
ROSETTA_BINFMT = CONFIG.install.rosetta_binfmt


def _container(**kwargs) -> tuple[InstallContainer, RecordingRunner]:
    runner = RecordingRunner(
        PROJECT_ROOT, replies={"systemctl is-system-running": "running"}, **kwargs
    )
    return InstallContainer(runner, sleep=lambda _seconds: None), runner


def _on(monkeypatch: pytest.MonkeyPatch, system: str) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: system)


# ---------------------------------------------------------------------------
# Host capability
# ---------------------------------------------------------------------------


def test_a_linux_host_with_virtualisation_devices_boots_a_guest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Linux")
    monkeypatch.setattr("capsem.gate.host.device_available", lambda path: path != "/dev/vsock")
    container, runner = _container()

    options = container.runtime_options()

    assert container.boots_a_guest
    assert "--device" in options and "/dev/kvm" in options
    assert "/dev/vhost-vsock" in options
    assert "/dev/vsock" not in options, "absent optional device must not be passed"
    assert "--group-add" not in options

    container.start(options=options)
    started = runner.matching(r"docker run -d")[0]
    assert (
        f"bash {CONFIG.install.vm_device_setup_script} {CONFIG.install.guest_user.name} "
        f"{CONFIG.install.systemd_command} /dev/kvm /dev/vhost-vsock"
    ) in started


def test_an_available_vsock_device_is_passed_through(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Linux")
    monkeypatch.setattr("capsem.gate.host.device_available", lambda _path: True)
    container, _ = _container()

    assert "/dev/vsock" in container.runtime_options()


def test_a_linux_host_without_kvm_refuses_rather_than_proving_less(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """On Linux the guest boot is the point; skipping it quietly is worse than
    failing, because the gate would then report a pass for a proof it did not
    run."""
    _on(monkeypatch, "Linux")
    monkeypatch.setattr("capsem.gate.host.device_available", lambda path: path != "/dev/kvm")
    container, _ = _container()

    with pytest.raises(GateError, match="/dev/kvm"):
        container.runtime_options()


def test_a_macos_host_proves_packaging_without_a_guest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    container, _ = _container()

    options = container.runtime_options()

    assert not container.boots_a_guest
    assert options == ["--security-opt", "seccomp=unconfined"]


# ---------------------------------------------------------------------------
# Rosetta
# ---------------------------------------------------------------------------


def test_rosetta_is_not_consulted_without_colima(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: None)
    container, runner = _container()

    container.require_rosetta()
    container.verify_rosetta_survived()

    assert not runner.ran("colima")


def test_a_missing_registration_stops_the_run_before_the_container(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/local/bin/colima")
    container, _ = _container(failures=[ROSETTA_BINFMT])

    with pytest.raises(GateError, match="missing before test-install"):
        container.require_rosetta()


def test_a_registration_removed_by_the_container_is_attributed_to_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The damage outlives the run, so the run has to be the one to report it."""
    _on(monkeypatch, "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/local/bin/colima")
    container, runner = _container()

    container.require_rosetta()
    runner.fail_on(ROSETTA_BINFMT)

    with pytest.raises(GateError, match="removed Colima's Rosetta"):
        container.verify_rosetta_survived()


def test_a_stopped_colima_is_not_a_failure(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _on(monkeypatch, "Darwin")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/local/bin/colima")
    container, _ = _container(failures=["colima status"])

    container.require_rosetta()
    container.verify_rosetta_survived()


# ---------------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------------


def test_a_predecessor_is_removed_before_the_container_starts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    container, runner = _container()

    container.start(options=[])

    runner.assert_order(r"docker rm -f", r"docker run -d")


def test_only_cgroups_are_mounted(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _on(monkeypatch, "Darwin")
    container, runner = _container()

    container.start(options=[])

    started = runner.matching(r"docker run -d")[0]
    assert "-v /sys/fs/cgroup:/sys/fs/cgroup:rw" in started
    # And not the checkout. The install image carries its source now, so a
    # mount of the working tree here would put the container back on the same
    # inodes as every concurrent host step.
    assert f"-v {PROJECT_ROOT}:" not in started, f"the checkout is mounted again: {started}"
    assert "--privileged --cgroupns=host" in started


def test_selected_release_transport_is_mounted_read_only_at_its_absolute_address(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Rewritten file URLs stay resolvable without mounting the checkout."""
    _on(monkeypatch, "Darwin")
    root = tmp_path / "selected"
    content = SelectedInstallContent(ProfileContent.isolated(CONFIG, root))
    runner = RecordingRunner(PROJECT_ROOT, replies={"systemctl is-system-running": "running"})
    container = InstallContainer(runner, content=content, sleep=lambda _seconds: None)

    container.start(options=[])

    started = runner.matching(r"docker run -d")[0]
    resolved = root.resolve()
    assert f"-v {resolved}:{resolved}:ro" in started
    assert f"-v {resolved / CONFIG.assets.merged_assets_dir}:/src/assets:ro" in started
    assert f"-v {resolved / CONFIG.assets.merged_config_dir}:/src/target/config:ro" in started


def _complete_selected_content(tmp_path: Path) -> SelectedInstallContent:
    root = tmp_path / "selected"
    selected = SelectedInstallContent(ProfileContent.isolated(CONFIG, root))
    inputs = selected.inputs(CONFIG)
    inputs.mkdir(parents=True)
    selected_manifest = {
        "channel": "stable",
        "profiles": {
            "code": {
                "architectures": [],
                "url": "https://release.capsem.test/profiles/code/profile.toml",
            }
        },
    }
    runtime_manifest = {
        "assets": {
            "current": "selected",
            "releases": {
                "selected": {"arches": {CONFIG.host_arch().name: {}}},
            },
        },
    }
    selected_encoded = json.dumps(selected_manifest).encode()
    runtime_encoded = json.dumps(runtime_manifest).encode()
    (inputs / CONFIG.package.release_inputs_name).write_text("{}")
    (inputs / CONFIG.install.manifest_name).write_bytes(selected_encoded)
    selected.content.assets.mkdir(parents=True)
    (selected.content.assets / CONFIG.install.manifest_name).write_bytes(runtime_encoded)
    materialize_required_artifacts(
        CONFIG,
        selected.content.assets,
        arches=(CONFIG.host_arch(),),
    )
    config_manifest = selected.content.config / CONFIG.suites.pytest.test_manifest
    config_manifest.parent.mkdir(parents=True)
    config_manifest.write_bytes(runtime_encoded)
    profile = selected.content.profiles(CONFIG) / "code/profile.toml"
    profile.parent.mkdir(parents=True)
    profile.write_text("name = 'code'\n")
    return selected


def test_selected_release_transport_accepts_the_published_graph_for_shared_verification(
    tmp_path: Path,
) -> None:
    selected = _complete_selected_content(tmp_path)
    manifest = selected.inputs(CONFIG) / CONFIG.install.manifest_name

    selected.require_complete(CONFIG, arches=(CONFIG.host_arch(),))

    document = json.loads(manifest.read_text())
    assert document["profiles"]["code"]["url"].startswith("https://release.capsem.test/")


def test_selected_release_transport_is_distinct_from_the_runtime_projection(
    tmp_path: Path,
) -> None:
    selected = _complete_selected_content(tmp_path)

    selected.require_complete(CONFIG, arches=(CONFIG.host_arch(),))

    transport = selected.inputs(CONFIG) / CONFIG.install.manifest_name
    runtime = selected.content.assets / CONFIG.install.manifest_name
    assert transport.read_bytes() != runtime.read_bytes()
    assert (
        runtime.read_bytes()
        == (selected.content.config / CONFIG.suites.pytest.test_manifest).read_bytes()
    )


def test_systemd_that_never_comes_up_fails_with_the_wait_it_gave(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    runner = RecordingRunner(PROJECT_ROOT, replies={"systemctl": "activating"})
    container = InstallContainer(runner, sleep=lambda _seconds: None)

    with pytest.raises(GateError, match="never reached running or degraded"):
        container.start(options=[])


def test_a_degraded_system_is_accepted(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """A container with one failed unit still installs packages; refusing it
    would fail the gate on something it does not test."""
    _on(monkeypatch, "Darwin")
    runner = RecordingRunner(PROJECT_ROOT, replies={"systemctl": "degraded"})
    container = InstallContainer(runner, sleep=lambda _s: None)

    container.start(options=[])

    assert runner.ran(r"chown -R capsem:capsem")


def test_only_the_target_directory_entry_is_granted_not_its_contents(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`rm -rf target/install-test-*` needs write permission on the parent
    entry, not on the entries themselves. A recursive chown here would walk
    every cargo artifact in the checkout."""
    _on(monkeypatch, "Darwin")
    container, runner = _container()

    container.start(options=[])

    assert runner.ran(r"chown capsem:capsem /src/target$")
    assert not runner.ran(r"chown -R capsem:capsem /src/target$")


def test_writes_are_handed_back_to_the_host_user(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    container, runner = _container()

    container.return_paths()

    uid, gid = os.getuid(), os.getgid()
    owned = CONFIG.install.layout.owned_paths(CONFIG.install.mount)
    assert runner.ran(rf"chown -R {uid}:{gid} " + owned[0])


def test_handing_paths_back_survives_a_container_that_already_died(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """It runs from cleanup, where the container may be gone; a failure here
    would replace the error the operator actually needs to read."""
    _on(monkeypatch, "Darwin")
    container, _ = _container(failures=["chown"])

    container.return_paths()


# ---------------------------------------------------------------------------
# The image the container runs
# ---------------------------------------------------------------------------


def test_the_image_is_always_rebuilt_then_smoked(tmp_path: Path) -> None:
    """Checking whether the tag exists lets a stale local image hide a new CI
    prerequisite, and then the gate proves an environment nobody else has."""
    from capsem.gate import installimage

    runner = RecordingRunner(PROJECT_ROOT)

    installimage.prepare(runner)

    runner.assert_order(
        r"docker build -t capsem-install-test",
        r"docker run --rm",
    )
    assert not runner.ran(r"--no-cache")
    # And it reclaims nothing on the way out. It used to release the parity
    # lane's builder rail from here, which was ordered only by the line it sat
    # on -- so once this preflight moved ahead of that lane, the release landed
    # 164ms before `cache-ownership` ran the image it had just deleted.
    assert not runner.ran(r"docker-storage-policy\.py release"), (
        "the preflight releases another lane's rail; that rail's own step does"
    )


def test_install_dependency_materialization_is_the_only_network_open_phase() -> None:
    settings = CONFIG.install

    assert settings.builder.materialize_build_network == "default"
    assert settings.builder.source_build_network == "none"
    assert settings.smoke_network == "none"
    assert settings.runtime_network == "none"


def test_install_helper_materializes_locked_inputs_before_the_sealed_image(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem.gate import installbuilder, installimage

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    runner = RecordingRunner(
        PROJECT_ROOT,
        failures=("docker image inspect capsem-install-builder:",),
    )

    identity = installbuilder.materialize(runner, CONFIG)
    installimage.build_source_image(runner, CONFIG, identity=identity)

    helper = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.install-builder") for value in command.argv)
    )
    source = next(
        command
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.install-test") for value in command.argv)
    )
    assert "--platform linux/amd64" in str(helper)
    assert "--network default" in str(helper)
    assert f"BASE=capsem-host-builder@sha256:{'0' * 64}" in str(helper)
    assert f"APT_SNAPSHOT_BASE={CONFIG.apt_snapshot.base}" in str(helper)
    assert f"APT_SNAPSHOT_ID={CONFIG.apt_snapshot.id}" in str(helper)
    assert f"RUST_TARGET={CONFIG.host_arch().rust_target}" in str(helper)
    assert f"CARGO_STORE={CONFIG.install.builder.cargo_store}" in str(helper)
    assert "INPUT_IDENTITY=capsem-install-builder:" in str(helper)
    assert "--platform linux/amd64" in str(source)
    assert "--network none" in str(source)
    assert f"BASE={identity.input_key}" in str(source)
    assert f"FRESH_CLI={CONFIG.install.source_cli}" in str(source)
    assert identity.image_id == "sha256:" + "0" * 64
    assert runner.last_index_of(
        r"docker image inspect --format '\{\{\.Os\}\}/\{\{\.Architecture\}\}.*"
        r"capsem-host-builder@sha256:"
    ) > runner.index_of(r"docker build .*Dockerfile\.install-builder")
    assert runner.last_index_of(
        r"--format '\{\{\.Os\}\}/\{\{\.Architecture\}\}.*capsem-install-builder:"
    ) > runner.index_of(r"docker build .*Dockerfile\.install-test")


def test_install_helper_accepts_a_local_parent_without_repository_digests(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem.gate import installbuilder

    monkeypatch.setattr("capsem.gate.host.machine", lambda: "x86_64")
    runner = RecordingRunner(
        PROJECT_ROOT,
        failures=("docker image inspect capsem-install-builder:",),
        replies={"{{json .RepoDigests}}": "[]"},
    )

    identity = installbuilder.materialize(runner, CONFIG)

    helper = next(
        str(command)
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.install-builder") for value in command.argv)
    )
    assert "BASE=capsem-host-builder:latest" in helper
    assert identity.image_reference == identity.input_key


def test_install_source_image_and_smoke_are_sealed_without_retry() -> None:
    from capsem.gate import installimage

    runner = RecordingRunner(PROJECT_ROOT)

    installimage.prepare(runner)

    source = next(
        str(command)
        for command in runner.commands
        if command.argv[:2] == ("docker", "build")
        and any(value.endswith("/Dockerfile.install-test") for value in command.argv)
    )
    smoke = next(
        str(command) for command in runner.commands if command.argv[:3] == ("docker", "run", "--rm")
    )
    assert "--network none" in source
    assert "--network none" in smoke
    assert "capsem-install-test:" in smoke
    assert "sha256:" not in smoke, (
        "containerd platform-child IDs are evidence, not runnable local image references"
    )
    assert "--no-cache" not in "\n".join(map(str, runner.commands))


def test_generated_asset_selector_identity_is_stable(tmp_path: Path) -> None:
    """The exact image built before assets assemble is the one install uses."""
    from capsem.gate import installimage

    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    (tmp_path / ".gitignore").write_bytes((PROJECT_ROOT / ".gitignore").read_bytes())
    script = tmp_path / CONFIG.candidate.source_digest_script
    script.parent.mkdir(parents=True)
    script.write_bytes((PROJECT_ROOT / CONFIG.candidate.source_digest_script).read_bytes())
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("source\n", encoding="utf-8")
    subprocess.run(
        ("git", "add", ".gitignore", str(script.relative_to(tmp_path)), "tracked.txt"),
        cwd=tmp_path,
        check=True,
    )
    config = CONFIG.model_copy(update={"root": tmp_path})

    before = installimage.source_image_tag(config, helper_id="sha256:helper")
    selected = tmp_path / "target" / "ironbank-assets" / "code" / "assets"
    selected.mkdir(parents=True)
    (tmp_path / "assets").symlink_to("target/ironbank-assets/code/assets")

    assert installimage.source_image_tag(config, helper_id="sha256:helper") == before


def test_the_install_image_is_built_after_the_builder_it_derives_from() -> None:
    """The other half of the claim above, now that the builder is a step.

    `prepare` used to run `just _build-host-image` itself -- a recipe that has
    never existed, so this path failed at that line every time and the test
    above proved only that the attempt was made in the right order. The
    dependency is an edge now, so it is checkable rather than merely attempted.
    """
    import argparse

    from capsem.gate import (
        cli,  # noqa: F401 - registers every command
        hostimage,
    )
    from capsem.gate.command import GateCommand

    plan = GateCommand.registry["install-image"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()

    assert (hostimage.STEP, "install.capacity") in plan.edges
    assert ("install.capacity", "install.materialize") in plan.edges
    assert ("install.materialize", "install.image-build") in plan.edges
    assert ("install.image-build", "install.image-smoke") in plan.edges


def test_a_failing_sealed_smoke_check_is_not_repaired_by_a_second_build() -> None:
    """Missing materialized input is a defect, not permission to fetch again."""
    from capsem.gate import installimage

    runner = RecordingRunner(PROJECT_ROOT, failures=["docker run --rm"])

    with pytest.raises(GateError, match="sealed image"):
        installimage.prepare(runner)

    assert len(runner.matching(r"docker run --rm")) == 1
    assert not runner.ran(r"--no-cache")


def test_the_virtualisation_devices_are_reachable_from_inside(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Passing `--device` is not proof the container can use it; a container
    that starts without working KVM fails much later, inside a VM boot."""
    _on(monkeypatch, "Linux")
    monkeypatch.setattr("capsem.gate.host.device_available", lambda _path: True)
    container, runner = _container()

    container.start(options=container.runtime_options())

    user = CONFIG.install.guest_user.name
    assert runner.ran(rf"docker exec -u {user} .*test -r /dev/kvm -a -w /dev/kvm")
    assert runner.ran(rf"docker exec -u {user} .*test -r /dev/vhost-vsock -a -w /dev/vhost-vsock")
    runner.assert_order(
        r"systemctl is-system-running --wait",
        rf"docker exec -u {user} .*test -r /dev/vhost-vsock",
    )


def test_missing_device_group_is_created_before_systemd(
    tmp_path: Path,
) -> None:
    """An unknown host gid is the ordinary Docker/Colima case, not an error.

    Keep the test outside a container by replacing only the account tools; the
    checked-in wrapper still owns validation, ordering, and the final exec.
    """
    commands = tmp_path / "bin"
    commands.mkdir()
    log = tmp_path / "calls.log"
    for name, body in {
        "id": "exit 0",
        "runuser": (
            'count=$(cat "$STATE" 2>/dev/null || printf 0); '
            'if [[ "$count" = 0 ]]; then printf 1 > "$STATE"; exit 1; fi'
        ),
        "stat": 'if [[ "$*" = *"%g"* ]]; then printf "4242\\n"; else printf "660\\n"; fi',
        "getent": "exit 2",
        "groupadd": 'printf "groupadd %s\\n" "$*" >> "$CALL_LOG"',
        "usermod": 'printf "usermod %s\\n" "$*" >> "$CALL_LOG"',
        "systemd": 'printf "systemd\\n" >> "$CALL_LOG"',
    }.items():
        command = commands / name
        command.write_text(f"#!/usr/bin/env bash\n{body}\n", encoding="utf-8")
        command.chmod(0o755)

    env = {
        **os.environ,
        "PATH": f"{commands}:{os.environ['PATH']}",
        "CALL_LOG": str(log),
        "STATE": str(tmp_path / "runuser-state"),
    }
    subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / CONFIG.install.vm_device_setup_script),
            CONFIG.install.guest_user.name,
            str(commands / "systemd"),
            "/dev/null",
        ],
        check=True,
        env=env,
    )

    assert log.read_text(encoding="utf-8").splitlines() == [
        "groupadd --gid 4242 capsem-vm-4242",
        f"usermod --append --groups capsem-vm-4242 {CONFIG.install.guest_user.name}",
        "systemd",
    ]


def test_the_install_containers_tmpfs_can_execute_what_is_unpacked_into_it() -> None:
    """`--tmpfs /tmp` alone is `noexec`, and the proof unpacks a binary there.

    The install proof extracts the shipped package into `/tmp` and runs its
    `capsem-admin` to author the release graph -- deliberately, so the graph is
    written by the exact binary being shipped rather than by whatever was on
    the image. Docker's default tmpfs flags are `rw,nosuid,nodev,noexec`, so
    `test -x` on the unpacked binary returns false and the proof fails with no
    output at all: `test` says nothing when it says no.

    The Linux-Rust container already spells this out, for the same reason and
    with the same comment. This is the other one.
    """
    from capsem.gate import config as gate_config

    paths = gate_config.load(PROJECT_ROOT).install.tmpfs_paths

    for entry in paths:
        assert ":" in entry, (
            f"{entry!r} takes Docker's default tmpfs flags, which include "
            "noexec; spell the options out"
        )
        mount, options = entry.split(":", 1)
        assert "exec" in options.split(","), (
            f"{mount} is mounted noexec; anything unpacked there cannot run"
        )
