"""Docker flags decided once, instead of re-decided at twenty call sites."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.docker import Docker
from capsem_builder.gate.dockermount import Mount, container_path
from capsem_builder.gate.errors import GateError
from capsem_builder.policy.dockerpolicy import ContainerNetwork
from helpers.gate import RecordingRunner

MOUNT = gate_config.load(Path(__file__).resolve().parents[3]).install.mount

# ---------------------------------------------------------------------------
# Containers
# ---------------------------------------------------------------------------


def test_removing_a_container_tolerates_its_absence(tmp_path: Path) -> None:
    """A stable name plus an unconditional removal is how a run recovers from
    a predecessor that died before its own cleanup."""
    runner = RecordingRunner(tmp_path, failures=["docker rm"])

    Docker(runner).remove("capsem-install-test")

    assert runner.matching(r"docker rm -f -v capsem-install-test")


def test_exec_builds_flags_in_the_order_docker_requires(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    Docker(runner).exec(
        "box", ["ls", "/src"], user="capsem", env={"XDG_RUNTIME_DIR": "/run/user/1000"}
    )

    assert runner.rendered[0] == (
        "docker exec -u capsem -e XDG_RUNTIME_DIR=/run/user/1000 box ls /src"
    )


def test_detached_exec_is_flagged_before_the_container(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    Docker(runner).shell("box", "serve", detach=True, user="capsem")

    assert runner.rendered[0].startswith("docker exec -d -u capsem box bash -c")


def test_shell_prepends_the_working_directory_once(tmp_path: Path) -> None:
    """Nearly every call site opened with `cd /src && `, and one did not."""
    runner = RecordingRunner(tmp_path)

    Docker(runner).shell("box", "pnpm install", cwd="/src/build_system/release_site")

    assert "cd /src/build_system/release_site && pnpm install" in runner.rendered[0]


def test_a_shell_fragment_keeps_its_quoting_intact(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    Docker(runner).shell("box", 'test -f "/src/a file.json"')

    assert runner.commands[0].argv[-1] == 'test -f "/src/a file.json"'


def test_mounts_render_in_docker_order(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)

    Docker(runner).run_detached(
        name="box",
        image="capsem-install-test",
        command=["/usr/lib/systemd/systemd"],
        network=ContainerNetwork.BRIDGE,
        options=["--privileged"],
        mounts=[
            Mount("/sys/fs/cgroup", "/sys/fs/cgroup", "rw"),
            Mount(str(tmp_path), "/src"),
        ],
    )

    assert runner.rendered[0] == (
        f"docker run -d --name box --network bridge --privileged "
        f"-v /sys/fs/cgroup:/sys/fs/cgroup:rw -v {tmp_path}:/src "
        f"capsem-install-test /usr/lib/systemd/systemd"
    )


# ---------------------------------------------------------------------------
# Path translation
# ---------------------------------------------------------------------------


def test_a_checkout_path_maps_onto_the_bind_mount(tmp_path: Path) -> None:
    mapped = container_path(
        tmp_path,
        tmp_path / "target" / "packages" / "Capsem_9.9.9_arm64.deb",
        mount=MOUNT,
    )

    assert mapped == f"{MOUNT}/target/packages/Capsem_9.9.9_arm64.deb"


def test_a_path_outside_the_checkout_is_refused(tmp_path: Path) -> None:
    """The shell built this with `${DEB#$ROOT/}`, which silently leaves an
    absolute host path in place when the prefix does not match -- so the
    container was handed a path that exists only on the host."""
    with pytest.raises(GateError, match="outside the mounted checkout"):
        container_path(tmp_path, Path("/elsewhere/Capsem.deb"), mount=MOUNT)


def test_capture_returns_container_stdout(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path, replies={"dpkg-deb -f": "9.9.9"})

    assert Docker(runner).capture("box", ["dpkg-deb", "-f", "x.deb", "Version"]) == "9.9.9"


def test_shell_capture_reads_a_value_out_of_the_container(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path, replies={"base_url": "http://127.0.0.1:8931"})

    served = Docker(runner).shell_capture(
        "box", "python3 -c 'print(base_url)'", user="capsem", cwd="/src"
    )

    assert served == "http://127.0.0.1:8931"
    assert "cd /src &&" in runner.rendered[0]


def test_exists_answers_without_failing_the_run(tmp_path: Path) -> None:
    """The handoff target must be checked, not assumed: an absent manifest is
    silently ignored by the installer, which then falls back to the public URL
    with no error at all."""
    runner = RecordingRunner(tmp_path, failures=["absent.json"])
    docker = Docker(runner)

    assert docker.exists("/src/target/manifest.json", "box")
    assert not docker.exists("/src/absent.json", "box")


def test_exact_image_presence_is_checked_for_its_platform(tmp_path: Path) -> None:
    runner = RecordingRunner(
        tmp_path,
        replies={"{{.Os}}/{{.Architecture}}": f"linux/arm64\tsha256:{'0' * 64}"},
    )
    image = "registry.example/guest@sha256:" + "a" * 64

    assert Docker(runner).image_exists(image, platform="linux/arm64")
    assert runner.rendered[0] == f"docker image inspect {image}"
    assert "--platform" not in runner.rendered[1]
    assert "{{.Os}}/{{.Architecture}}" in runner.rendered[1]


def test_exact_image_pull_names_platform_and_digest(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)
    image = "registry.example/guest@sha256:" + "b" * 64

    Docker(runner).pull(image, platform="linux/amd64")

    assert runner.rendered == [f"docker pull --platform linux/amd64 {image}"]
