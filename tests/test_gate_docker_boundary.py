"""Containers get their source copied in, not the developer's checkout mounted.

Every gate container does `-v <repo_root>:/src`, which is two defects wearing
one flag.

It is an isolation hole: the container can read and often write the live
checkout, so "the gate runs in a container" says nothing about what the
container can reach.

And it is a *race*. `rust-coverage` runs on the host and churns hardlinks
inside that tree while `linux-rust` reads the same inodes through virtiofs.
A release run died on it -- `Permission denied` opening
`config/profiles/code/root/root/.gemini/projects.json`, a file that was `0644`
before and `0644` after -- and no amount of declaring `contends` would have
prevented it, because the two steps genuinely share nothing except the
filesystem nobody wrote down.

Copying the source into an image removes the class rather than guarding it.
There is no mount to police, no host path to rewrite, and nothing for two
steps to contend over: each container holds its own bytes.

These are argv-level assertions on purpose. The mount is a flag, the network
mode is a flag, and a flag is what a future change will quietly reintroduce.
"""

from __future__ import annotations

import ast
import inspect
from pathlib import Path
from typing import Any, cast, get_type_hints

import pytest
from capsem_builder.policy.dockerpolicy import BuildNetwork, ContainerNetwork
from helpers.gate import RecordingRunner

from capsem.gate.docker import Docker
from capsem.gate.errors import GateError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
GATE = PROJECT_ROOT / "src" / "capsem" / "gate"

#: The one module allowed to spell `docker` as a command name. Everything else
#: asks it, so that mounts and network mode have a single place to be decided.
WRAPPER = {"docker.py", "dockerimage.py"}


def _modules() -> list[Path]:
    return sorted(path for path in GATE.glob("*.py") if path.name not in WRAPPER)


def _docker_argv_literals(tree: ast.AST) -> list[ast.List]:
    """Every list literal whose first element is the string `docker`."""
    found: list[ast.List] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.List) or not node.elts:
            continue
        first = node.elts[0]
        if isinstance(first, ast.Constant) and first.value == "docker":
            found.append(node)
    return found


#: Modules that still build docker argv by hand. A ratchet rather than an
#: `xfail`: an `xfail` says "this is broken" and hides how broken, whereas
#: this refuses a tenth site while the nine are migrated. Each removal here is
#: a module that can no longer choose its own mount or network mode.
#:
#: These are exact counts, not ceilings -- see
#: `test_the_ratchet_carries_no_slack`. Started at nine; `hostimage.py` sat at
#: 5 against an actual 2 for long enough that the guard would have permitted
#: three new hand-built sites in the module Phase 5 was about to touch.
#: Empty. Every module asks the wrapper now, so the mount, the network mode and
#: the removal policy are decided in one place -- which is what the ratchet was
#: for. Kept as a table rather than deleted with the guard, because the guard
#: is what keeps it empty: a new module building `docker` argv fails
#: `test_no_new_module_builds_docker_argv_by_hand` outright.
UNMIGRATED: dict[str, int] = {}


def test_no_new_module_builds_docker_argv_by_hand() -> None:
    """One place decides the flags, or every call site decides them again.

    The mount, the network mode and the removal policy have to be identical
    everywhere; spread across four modules they were made differently in nine
    places -- including the `-v <repo_root>:/src` that raced a host step and
    killed a release run.
    """
    counted: dict[str, int] = {}
    for path in _modules():
        tree = ast.parse(path.read_text(encoding="utf-8"))
        found = _docker_argv_literals(tree)
        if found:
            counted[path.name] = len(found)

    new = {name: count for name, count in counted.items() if name not in UNMIGRATED}
    assert not new, f"new modules building docker argv directly: {new}"

    grown = {
        name: (count, UNMIGRATED[name])
        for name, count in counted.items()
        if count > UNMIGRATED[name]
    }
    assert not grown, f"these grew new hand-built docker argv (now, allowed): {grown}"

    # And the debt only shrinks: a migrated module leaves the list.
    assert set(counted) <= set(UNMIGRATED), sorted(set(counted) - set(UNMIGRATED))


def test_the_ratchet_carries_no_slack() -> None:
    """`UNMIGRATED` records what is there, not what would be tolerated.

    Without this, paying the debt down and leaving the number alone is
    invisible, and the gap silently becomes an allowance for new sites. That
    is not hypothetical: the table said `hostimage.py: 5` against an actual 2,
    so three hand-built `docker` argv could have been added to the module Phase
    5 is about to rewrite, and every guard here would have stayed green.

    The cost is one line per migration -- lower the count in the same commit --
    and the benefit is that the ratchet cannot quietly stop being one.
    """
    counted: dict[str, int] = {}
    for path in _modules():
        found = _docker_argv_literals(ast.parse(path.read_text(encoding="utf-8")))
        if found:
            counted[path.name] = len(found)

    slack = {
        name: (allowed, counted.get(name, 0))
        for name, allowed in UNMIGRATED.items()
        if allowed > counted.get(name, 0)
    }
    assert not slack, (
        "these carry more allowance than debt (allowed, actual) -- lower them "
        f"to the actual count, or drop the entry entirely at zero: {slack}"
    )


def test_a_mount_cannot_point_at_the_checkout() -> None:
    """The specific hole: `-v <repo_root>:/src`.

    Refused at construction rather than reviewed, because this is the flag
    that made a host step and a container step share inodes.
    """
    import pytest

    from capsem.gate import config as gate_config
    from capsem.gate.dockermount import Mount
    from capsem.gate.errors import GateError

    root = gate_config.load(PROJECT_ROOT).root

    with pytest.raises(GateError, match="checkout"):
        Mount(source=str(root), target="/src")

    with pytest.raises(GateError, match="checkout"):
        Mount(source=str(root / "config"), target="/src/config")

    # A named volume and a path outside the checkout stay legal: this refuses
    # the checkout, not mounting.
    assert Mount(source="capsem-cargo-registry", target="/usr/local/cargo/registry")
    assert Mount(source="/tmp/capsem-scratch", target="/scratch")


def test_every_container_declares_its_network() -> None:
    """No default, so `--network` is a decision rather than an omission.

    Nothing in the gate passes `--network` today, which means every container
    has outbound access and several use it mid-run. A required keyword makes
    that visible at each call site instead of invisible everywhere.
    """
    for name in ("read", "run_detached", "run_once", "probe", "create"):
        method = getattr(Docker, name, None)
        assert method is not None, f"Docker.{name} is missing"
        parameter = inspect.signature(method).parameters.get("network")
        assert parameter is not None, f"Docker.{name} does not take a network mode"
        assert parameter.default is inspect.Parameter.empty, (
            f"Docker.{name} defaults its network mode, so a call site can omit "
            "the decision and get outbound access without saying so"
        )
        assert get_type_hints(method)["network"] is ContainerNetwork

    build_network = inspect.signature(Docker.build).parameters.get("network")
    assert build_network is not None
    assert build_network.default is inspect.Parameter.empty
    assert get_type_hints(Docker.build)["network"] is BuildNetwork


def test_build_network_rejects_a_container_only_mode(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)
    docker = Docker(runner)

    with pytest.raises(TypeError, match="BuildNetwork enum"):
        docker.build(
            tag="invalid-build",
            dockerfile="Dockerfile",
            context=".",
            network=cast(Any, ContainerNetwork.BRIDGE),
        )

    assert runner.commands == []


def test_image_reference_refuses_a_digest_for_a_different_repository(tmp_path: Path) -> None:
    runner = RecordingRunner(
        tmp_path,
        replies={"{{json .RepoDigests}}": f'["other@sha256:{"a" * 64}"]'},
    )

    with pytest.raises(GateError, match="matching repository digest"):
        Docker(runner).image_reference("capsem-host-builder:latest")


def test_exact_build_reference_accepts_a_locally_built_image_without_repo_digests(
    tmp_path: Path,
) -> None:
    from capsem.gate.imageidentity import exact_image_reference

    runner = RecordingRunner(
        tmp_path,
        replies={
            "{{json .RepoDigests}}": "[]",
            "{{.Os}}/{{.Architecture}}": f"linux/amd64\tsha256:{'0' * 64}",
        },
    )

    assert (
        exact_image_reference(
            Docker(runner),
            "capsem-host-builder:latest",
            platform="linux/amd64",
            expected_id="sha256:" + "0" * 64,
            subject="local build",
        )
        == "capsem-host-builder:latest"
    )


def test_platform_image_identity_does_not_require_inspect_platform_flag(tmp_path: Path) -> None:
    runner = RecordingRunner(
        tmp_path,
        replies={"{{.Os}}/{{.Architecture}}": f"linux/amd64\tsha256:{'a' * 64}"},
    )

    assert Docker(runner).image_id("capsem-host-builder:latest", platform="linux/amd64") == (
        "sha256:" + "a" * 64
    )
    assert all("--platform" not in command.argv for command in runner.commands)


def test_platform_image_identity_refuses_the_wrong_platform(tmp_path: Path) -> None:
    runner = RecordingRunner(
        tmp_path,
        replies={"{{.Os}}/{{.Architecture}}": f"linux/arm64\tsha256:{'a' * 64}"},
    )

    with pytest.raises(GateError, match=r"expected platform linux/amd64.*linux/arm64"):
        Docker(runner).image_id("capsem-host-builder:latest", platform="linux/amd64")

    assert all("--platform" not in command.argv for command in runner.commands)


def test_runtime_identity_normalizes_docker_table_alignment(tmp_path: Path) -> None:
    runner = RecordingRunner(
        tmp_path,
        replies={"{{.Server.Version}}": "29.1.3              linux               amd64"},
    )

    assert Docker(runner).runtime_identity() == "29.1.3\tlinux\tamd64"


@pytest.mark.parametrize(
    "reported",
    ["29.1.3 linux", "29.1.3 linux amd64 extra", "29.1.3\nlinux amd64"],
)
def test_runtime_identity_refuses_an_ambiguous_shape(
    tmp_path: Path, reported: str
) -> None:
    runner = RecordingRunner(tmp_path, replies={"{{.Server.Version}}": reported})

    with pytest.raises(GateError, match="malformed runtime identity"):
        Docker(runner).runtime_identity()


@pytest.mark.parametrize("operation", ["read", "run_detached", "run_once", "probe", "create"])
def test_every_container_adapter_rejects_a_buildkit_only_mode(
    tmp_path: Path, operation: str
) -> None:
    runner = RecordingRunner(tmp_path)
    docker = Docker(runner)

    with pytest.raises(TypeError, match="ContainerNetwork enum"):
        if operation == "read":
            docker.read(
                image="invalid-run",
                command=["true"],
                network=cast(Any, BuildNetwork.DEFAULT),
            )
        elif operation == "run_detached":
            docker.run_detached(
                name="invalid-run",
                image="invalid-run",
                command=["true"],
                network=cast(Any, BuildNetwork.DEFAULT),
            )
        elif operation == "run_once":
            docker.run_once(
                image="invalid-run",
                command=["true"],
                network=cast(Any, BuildNetwork.DEFAULT),
            )
        elif operation == "probe":
            docker.probe(
                image="invalid-run",
                command=["true"],
                network=cast(Any, BuildNetwork.DEFAULT),
            )
        else:
            docker.create(
                name="invalid-run",
                image="invalid-run",
                command=["true"],
                network=cast(Any, BuildNetwork.DEFAULT),
            )

    assert runner.commands == []


def test_docker_network_boundaries_reject_raw_strings_before_issuing_commands(
    tmp_path: Path,
) -> None:
    runner = RecordingRunner(tmp_path)
    docker = Docker(runner)

    with pytest.raises(TypeError, match="BuildNetwork enum"):
        cast(Any, docker.build)(
            tag="raw-build", dockerfile="Dockerfile", context=".", network="none"
        )
    with pytest.raises(TypeError, match="ContainerNetwork enum"):
        cast(Any, docker.run_once)(image="raw-run", command=["true"], network="none")

    assert runner.commands == []


def test_typed_network_modes_render_their_exact_docker_vocabulary(tmp_path: Path) -> None:
    runner = RecordingRunner(tmp_path)
    docker = Docker(runner)

    docker.build(
        tag="sealed-build",
        dockerfile="Dockerfile",
        context=".",
        network=BuildNetwork.NONE,
    )
    docker.run_once(
        image="sealed-run",
        command=["true"],
        network=ContainerNetwork.NONE,
    )

    assert runner.commands[0].argv == (
        "docker",
        "build",
        "-t",
        "sealed-build",
        "-f",
        "Dockerfile",
        "--network",
        "none",
        ".",
    )
    assert runner.commands[1].argv == (
        "docker",
        "run",
        "--rm",
        "--network",
        "none",
        "sealed-run",
        "true",
    )


def test_no_module_mounts_the_checkout() -> None:
    """Zero, and it stays zero.

    This counted six, then five, and now none: every lane copies its source
    into its image, and `Mount.unmigrated` is deleted with its last caller. The
    guard is an absolute rather than a shrinking budget -- the constructor that
    permitted a checkout mount no longer exists to be called.

    What remains is `Mount.generated`, which is not the same thing and must not
    become it. It addresses build *output* a container reads: `assets/` is
    3.0 GB and changes every run, so copying it would put a multi-gigabyte
    layer in Docker storage per gate. Those inputs are produced by an earlier
    step and mounted read-only, while the race that killed a release run was a
    *source* path hardlink-churned by a concurrent host step.
    """
    offenders = {
        path.name: path.read_text(encoding="utf-8").count("Mount.unmigrated(")
        for path in _modules()
        if "Mount.unmigrated(" in path.read_text(encoding="utf-8")
    }
    assert offenders == {}, (
        f"a checkout mount is back: {offenders}. The constructor was deleted; a "
        "lane needing the working tree should COPY it, and one needing build "
        "output should say so with Mount.generated."
    )


def test_generated_mounts_are_read_only() -> None:
    """A lane that can write its inputs can change the next lane's inputs."""
    from capsem.gate.dockermount import Mount

    assert Mount.generated("/x/assets", "/src/assets").options == "ro"
