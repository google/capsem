"""The macOS-hosted Linux parity lane holds its own bytes.

It carried every defect this work exists for: it bind-mounted the live
checkout read-only, grafted two writable mounts through that to get output
back, depended on four named volumes that persist between runs, and ran with
outbound network because nothing ever passed `--network`. A release died in
this lane with `Permission denied` on a file that was `0644` before and after,
because `rust-coverage` was churning hardlinks in the tree it was reading.

Copying the source into an image removes all four at once. There is no mount
to race over, no volume to inherit, and the dependencies live in a base image
keyed by everything that defines it.
"""

from __future__ import annotations

from pathlib import Path, PurePosixPath

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _macos_issued(monkeypatch: pytest.MonkeyPatch) -> str:
    import sys

    sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: "arm64")
    from helpers.gate import gate_issued

    return gate_issued("linux-rust")


def test_the_lane_mounts_nothing(monkeypatch: pytest.MonkeyPatch) -> None:
    """No `-v` at all. Not a rewritten mount -- none."""
    issued = _macos_issued(monkeypatch)
    docker_lines = [line for line in issued.splitlines() if line.startswith("docker ")]
    assert docker_lines, f"no docker command was issued:\n{issued}"
    # Only the commands that can mount. `docker rm -f -v` also carries a `-v`,
    # and it means "take the anonymous volumes with the container" -- the
    # opposite of a mount, and reading it as one would fail this for the
    # teardown doing its job.
    mounting = [
        line
        for line in docker_lines
        if line.startswith(("docker create", "docker run")) and " -v " in line
    ]
    assert not mounting, "the lane still mounts something:\n  " + "\n  ".join(mounting)


def test_the_lane_runs_with_no_network(monkeypatch: pytest.MonkeyPatch) -> None:
    """It compiles and runs tests. Fetching mid-run is what the base image is
    for, and denying it is what proves the base image is complete."""
    issued = _macos_issued(monkeypatch)
    # `docker create` rather than `docker run`: the container has to outlive
    # its own exit so the coverage can be copied out of it.
    created = [line for line in issued.splitlines() if line.startswith("docker create")]
    assert created, f"the lane does not create a container:\n{issued}"
    for line in created:
        assert "--network none" in line, f"the lane can still reach the network: {line}"


PARENT = "sha256:1111111111111111111111111111111111111111111111111111111111111111"


def _staged(tmp_path: Path):
    """A checkout holding only what the base image's identity is made of.

    Small enough to mutate file by file, and real enough that `base_tag` reads
    it exactly as it reads the checkout.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    for name in _identity_files(config):
        source, target = config.path(name), tmp_path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
    return config.model_copy(update={"root": tmp_path})


def _identity_files(config) -> tuple[str, ...]:
    return (config.hostimage.base_dockerfile, *config.hostimage.identity_inputs)


def _docker(root: Path, parent: str):
    from helpers.gate import RecordingRunner, recorded_image_identity

    from capsem.gate.docker import Docker
    from capsem.gate.dockerimage import IMAGE_IDENTITY_FORMAT

    return Docker(
        RecordingRunner(
            root,
            replies={
                IMAGE_IDENTITY_FORMAT: recorded_image_identity(
                    root, "capsem-host-builder:latest", image_id=parent
                )
            },
        )
    )


def test_every_input_that_defines_the_base_image_changes_its_tag(tmp_path: Path) -> None:
    """`WarmBase` skips the build whenever the tag already exists, so anything
    missing from the key is an environment change the sealed lane never sees.

    The old version of this test only checked the configured lockfiles were
    present on disk, which is true of an incomplete key as well as a complete
    one. This mutates each defining input in turn and requires the tag to move
    -- including the two that were missing: the Dockerfile, which carries the
    ONNX Runtime version and every build argument's default, and the mutable
    `capsem-host-builder:latest` the image is `FROM`.
    """
    from capsem.gate import linuxrust

    config = _staged(tmp_path)
    docker = _docker(tmp_path, PARENT)
    baseline = linuxrust.base_tag(config, docker)
    assert ":" in baseline, baseline

    for name in _identity_files(config):
        path = config.path(name)
        original = path.read_bytes()
        path.write_bytes(original + b"\n# changed\n")
        try:
            assert linuxrust.base_tag(config, docker) != baseline, (
                f"{name} defines the base image and does not key it, so a "
                "change to it reuses the image built before it"
            )
        finally:
            path.write_bytes(original)

    assert linuxrust.base_tag(config, docker) == baseline, "the mutations did not restore"


def test_rebuilding_the_parent_image_changes_the_tag(tmp_path: Path) -> None:
    """`capsem-host-builder:latest` is a mutable tag, so the same name is a
    different image after `just warm` rebuilds it.

    Keyed by name alone, a rebuilt parent leaves the sealed lane testing
    against the toolchain, system packages and CA bundle of whatever the parent
    used to be -- the exact class of drift the base image exists to remove.
    """
    from capsem.gate import linuxrust

    config = _staged(tmp_path)
    before = linuxrust.base_tag(config, _docker(tmp_path, PARENT))
    after = linuxrust.base_tag(config, _docker(tmp_path, PARENT.replace("1", "2")))

    assert before != after


def test_the_lane_owns_its_base_image_instead_of_asking_the_operator(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A module owns its prerequisites, or `just test` is not self-sufficient.

    The lane refuses to build the base image inside itself -- correctly, since
    it runs sealed and a multi-gigabyte fetch mid-run is the thing sealing
    prevents. But refusing was the whole answer: the base image was warmed by a
    separate recipe the operator had to know to run, and `linux-rust` sits
    twenty-five minutes into the gate. A clean machine therefore spent
    twenty-five minutes to be handed a command it could have run first.

    Composed as a step before the lane, the refusal becomes unreachable in
    practice and the gate is what `AGENTS.md` says it is: every prerequisite
    owned, runnable in a clean local environment.
    """
    from helpers.gate import gate_labels

    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: "arm64")
    labels = list(gate_labels("candidate"))

    assert "warm-base" in labels, (
        "nothing in the gate builds the parity base image, so a clean machine "
        "reaches the lane and is told to run a recipe by hand"
    )
    assert labels.index("warm-base") < labels.index("linux-rust")


def test_the_ownership_steps_are_gone(monkeypatch: pytest.MonkeyPatch) -> None:
    """`cache-ownership` and `output-ownership` existed only because root-owned
    volumes and bind mounts left files the host could not read. Without either,
    they are ceremony -- and a ratchet keeps them from coming back."""
    from helpers.gate import gate_labels

    monkeypatch.setattr("capsem.gate.host.system", lambda: "Darwin")
    monkeypatch.setattr("capsem.gate.host.machine", lambda: "arm64")
    labels = set(gate_labels("test-static")) | set(gate_labels("linux-rust"))
    assert "cache-ownership" not in labels, sorted(labels)
    assert "output-ownership" not in labels, sorted(labels)


def test_the_coverage_output_is_copied_out_before_the_container_is_removed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """`--rm` and `docker cp` are mutually exclusive: a removed container has
    nothing left to copy from. The edge is the assertion."""
    issued = _macos_issued(monkeypatch)
    lines = issued.splitlines()

    def last_index_of(fragment: str) -> int:
        for position in reversed(range(len(lines))):
            if fragment in lines[position]:
                return position
        raise AssertionError(f"{fragment!r} was never issued:\n{issued}")

    # The *last* removal: the lane also removes a predecessor before creating
    # its own, and comparing against that one would pass while the teardown
    # still destroyed the evidence.
    assert last_index_of("docker cp") < last_index_of("docker rm"), issued


def test_the_build_context_is_bounded() -> None:
    """The image copies the source, so `.dockerignore` decides what "source" is.

    A bare `target` pattern matches only the repository root. This checkout
    carries agent worktrees under `.claude/` -- 55 GB of them, each with its
    own `target/` -- so the first build swept them in and died with `no space
    left on device` after nineteen minutes. Every exclusion is `**`-prefixed
    now, and this asserts the outcome rather than the patterns: a new cache
    directory nobody thought to exclude fails here in a second instead of
    filling the Docker disk.
    """
    import fnmatch

    patterns = [
        line.strip()
        for line in (PROJECT_ROOT / ".dockerignore").read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]

    def ignored(relative: str) -> bool:
        parts = relative.split("/")
        for pattern in patterns:
            bare = pattern.removeprefix("**/")
            if pattern.startswith("**/"):
                if any(fnmatch.fnmatch(segment, bare) for segment in parts):
                    return True
            elif fnmatch.fnmatch(parts[0], bare):
                return True
        return False

    total = 0
    for path in PROJECT_ROOT.rglob("*"):
        relative = str(path.relative_to(PROJECT_ROOT))
        if ignored(relative):
            continue
        try:
            if path.is_file():
                total += path.stat().st_size
        except OSError:
            continue

    megabytes = total / 1048576
    assert megabytes < 400, (
        f"the docker build context is {megabytes:.0f} MB. Something large is no "
        "longer excluded; a build with this context fills the Docker disk "
        "rather than failing fast."
    )


def test_the_lane_image_carries_no_release_credentials() -> None:
    """`COPY . /src` puts the build context into a tagged, retained image.

    Sealing the lane replaced a read-only bind mount with a copy, which is
    stronger for isolation and strictly worse for secrets: the mount exposed
    the checkout only while a container ran, whereas
    `capsem-linux-rust:latest` is tagged and kept between runs. The Tauri
    signing key, the Apple certificates and the minisign manifest key were
    therefore sitting in Docker storage after every gate.

    The lane compiles and runs Rust tests. It signs nothing.

    Derived from `[package.signing] directory` rather than a literal path, so
    moving the signing material cannot silently leave this guard checking
    somewhere nobody keeps keys. `security/keys/capsem-ca.key` is deliberately
    not covered: that is the MITM CA, committed and public by design, and the
    guest needs it.
    """
    from capsem.gate import config as gate_config

    config = gate_config.load(PROJECT_ROOT)
    ignored = (PROJECT_ROOT / ".dockerignore").read_text(encoding="utf-8").split()
    secrets = PurePosixPath(config.package.signing.directory).parts[0]

    assert f"**/{secrets}" in ignored or secrets in ignored, (
        f"{secrets}/ is not excluded from the Docker build context, so "
        f"`COPY . /src` bakes the release signing material into "
        f"{config.hostimage.lane_tag} -- which is tagged and outlives the run"
    )
