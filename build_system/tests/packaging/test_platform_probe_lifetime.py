"""Real signal delivery, recorded Docker calls; never executes Docker."""

import os
import signal
from pathlib import Path

import pytest
from capsem_builder.gate import cancellation
from capsem_builder.gate.docker import Docker
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.platformproof import probe_container
from capsem_builder.release.tools import deb_preunpack_probe as probe


def test_forced_client_interrupt_removes_only_the_exact_probe(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    package = tmp_path / "candidate.deb"
    package.write_bytes(b"source-only original package")
    removed = []

    def remove(_self, container, *, strict=False):
        removed.append((container, strict))

    def interrupted(*args, **kwargs):
        raise KeyboardInterrupt("controlled client interruption")

    monkeypatch.setattr(Docker, "remove", remove)
    monkeypatch.setattr(probe.subprocess, "run", interrupted)
    with pytest.raises(KeyboardInterrupt, match="controlled"):
        probe.check_preunpack("source-fixture", package, "arm64", True)
    assert len(removed) == 1
    assert removed[0][1] is True
    assert "-platform-" in removed[0][0]


def test_real_term_unwinds_exact_owned_name_and_restores_handler(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    package = tmp_path / "candidate.deb"
    package.write_bytes(b"source-only original package")
    names = []
    removed = []
    previous = signal.getsignal(signal.SIGTERM)

    def remove(_self, container, *, strict=False):
        removed.append((container, strict))

    def terminated(argv, **kwargs):
        names.append(argv[argv.index("--name") + 1])
        os.kill(os.getpid(), signal.SIGTERM)
        pytest.fail("TERM must unwind the probe")

    monkeypatch.setattr(Docker, "remove", remove)
    monkeypatch.setattr(probe.subprocess, "run", terminated)
    with pytest.raises(cancellation.Terminated):
        probe.check_preunpack("source-fixture", package, "arm64", True)
    assert removed == [(names[0], True)]
    assert signal.getsignal(signal.SIGTERM) == previous


def test_cleanup_failure_is_not_success_or_silent_absence(monkeypatch: pytest.MonkeyPatch) -> None:
    def failed_remove(_self, container, *, strict=False):
        assert strict
        raise GateError(f"exact removal failed: {container}")

    monkeypatch.setattr(Docker, "remove", failed_remove)
    with (
        pytest.raises(GateError, match=r"failed to release.*exact removal failed"),
        probe_container(Path(__file__).resolve().parents[3]),
    ):
        pass


def test_cleanup_failure_preserves_original_client_error(monkeypatch: pytest.MonkeyPatch) -> None:
    def failed_remove(_self, container, *, strict=False):
        assert strict
        raise GateError("controlled cleanup failure")

    monkeypatch.setattr(Docker, "remove", failed_remove)
    with (
        pytest.raises(KeyboardInterrupt, match="original client") as error,
        probe_container(Path(__file__).resolve().parents[3]),
    ):
        raise KeyboardInterrupt("original client")
    assert any("controlled cleanup failure" in note for note in error.value.__notes__)


def test_parallel_invocations_never_reuse_or_preempt_names(monkeypatch: pytest.MonkeyPatch) -> None:
    removed = []
    monkeypatch.setattr(
        Docker, "remove", lambda _self, name, *, strict=False: removed.append((name, strict))
    )
    root = Path(__file__).resolve().parents[3]
    with probe_container(root) as first:
        assert not removed
        with probe_container(root) as second:
            assert not removed
            assert first != second
        assert removed == [(second, True)]
    assert removed == [(second, True), (first, True)]


@pytest.mark.parametrize("entrypoint", ["planned", "local"])
@pytest.mark.parametrize("cached_platform", [None, "linux/other"])
def test_matrix_prepares_missing_platform_children_before_sealed_proof(
    monkeypatch, cached_platform, entrypoint
):
    from capsem_builder.gate.config import load
    from capsem_builder.gate.context import Context
    from capsem_builder.gate.platformproof import platform_step, prove
    from capsem_builder.gate.proc import Runner

    config = load(Path(__file__).resolve().parents[3])
    platform = config.host_arch().docker_platform
    cached = {row.image(): {cached_platform} for row in config.platforms.linux.distributions}
    proof_calls = []
    runner = Runner(config.root)

    def run(argv, **_kwargs):
        if tuple(argv[:2]) == ("docker", "pull"):
            assert list(argv[2:4]) == ["--platform", platform]
            cached[argv[-1]].add(platform)
        else:
            assert all(platform in children for children in cached.values()), (
                "a cached index or foreign child cannot satisfy the sealed platform proof"
            )
            proof_calls.append(argv)

    monkeypatch.setattr(runner, "run", run)
    context = Context(config=config, runner=runner)
    if entrypoint == "planned":
        for action in platform_step(config, "candidate.deb").actions:
            action.perform(context)
    else:
        prove(runner, config, Path("candidate.deb"))
    assert len(proof_calls) == 1
