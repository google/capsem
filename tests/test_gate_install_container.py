"""What the install container can prove on this host, and what it must refuse.

Two host facts decide the answer and both used to sit in the middle of a
270-line recipe: whether `/dev/kvm` and `/dev/vhost-vsock` are usable, and
whether Colima's Rosetta binfmt registration is present. The second is checked
twice because a privileged systemd container has removed it -- which breaks
every later x86 build on the machine, not just the run that caused it.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
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
    monkeypatch.setattr(
        "capsem.gate.host.device_available", lambda path: path != "/dev/vsock"
    )
    container, _ = _container()

    options = container.runtime_options()

    assert container.boots_a_guest
    assert "--device" in options and "/dev/kvm" in options
    assert "/dev/vhost-vsock" in options
    assert "/dev/vsock" not in options, "absent optional device must not be passed"


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
    monkeypatch.setattr(
        "capsem.gate.host.device_available", lambda path: path != "/dev/kvm"
    )
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


def test_a_stopped_colima_is_not_a_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
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


def test_the_checkout_and_cgroups_are_mounted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    container, runner = _container()

    container.start(options=[])

    started = runner.matching(r"docker run -d")[0]
    assert "-v /sys/fs/cgroup:/sys/fs/cgroup:rw" in started
    assert f"-v {PROJECT_ROOT}:/src" in started
    assert "--privileged --cgroupns=host" in started


def test_systemd_that_never_comes_up_fails_with_the_wait_it_gave(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _on(monkeypatch, "Darwin")
    runner = RecordingRunner(PROJECT_ROOT, replies={"systemctl": "activating"})
    container = InstallContainer(runner, sleep=lambda _seconds: None)

    with pytest.raises(GateError, match="never reached running or degraded"):
        container.start(options=[])


def test_a_degraded_system_is_accepted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
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
        r"docker-storage-policy\.py release --boundary after-linux-rust-builder",
    )
    assert not runner.ran(r"--no-cache")


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

    assert (hostimage.STEP, "install-image") in plan.edges


def test_a_failing_smoke_check_earns_exactly_one_cacheless_rebuild(
    tmp_path: Path,
) -> None:
    """A cached layer can satisfy `docker build` and still be missing a tool."""
    from capsem.gate import installimage

    class RepairedByRebuild(RecordingRunner):
        """The smoke check starts failing and stops once the cache is bypassed."""

        def execute(self, command):
            completed = super().execute(command)
            if "--no-cache" in str(command):
                self.fail_on()
            return completed

    runner = RepairedByRebuild(PROJECT_ROOT, failures=["docker run --rm"])

    installimage.prepare(runner)

    assert len(runner.matching(r"--no-cache")) == 1
    assert len(runner.matching(r"docker run --rm")) == 2, (
        "the image must be re-smoked after the rebuild, not assumed fixed"
    )


def test_a_smoke_check_that_fails_twice_is_a_dockerfile_defect(tmp_path: Path) -> None:
    from capsem.gate import installimage

    runner = RecordingRunner(PROJECT_ROOT, failures=["docker run --rm"])

    with pytest.raises(GateError, match="cannot run the install gate's tools"):
        installimage.prepare(runner)

    assert len(runner.matching(r"--no-cache")) == 1, "a third attempt proves nothing"


def test_the_virtualisation_devices_are_reachable_from_inside(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Passing `--device` is not proof the container can use it; a container
    that starts without working KVM fails much later, inside a VM boot."""
    _on(monkeypatch, "Linux")
    monkeypatch.setattr("capsem.gate.host.device_available", lambda _path: True)
    container, runner = _container()

    container.start(options=container.runtime_options())

    assert runner.ran(r"test -r /dev/kvm -a -w /dev/kvm")
    assert runner.ran(r"test -r /dev/vhost-vsock -a -w /dev/vhost-vsock")
