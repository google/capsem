"""Docker capacity pressure preserves a configured hot BuildKit cohort."""

from pathlib import Path

from capsem_builder.cache.capacity import ensure_capacity
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.runtimemodels import RuntimeCommandResult

from .test_runtime_control import controlled_policy


def result(argv: tuple[str, ...], output: str = "", returncode: int = 0):
    return RuntimeCommandResult(
        argv=argv,
        returncode=returncode,
        stdout=output,
        stderr="",
        duration_ms=1,
    )


def test_pressure_prunes_buildkit_then_remeasures(tmp_path: Path) -> None:
    issued = []
    capacities = iter(("200 200 0", "200 199 1"))

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        issued.append(argv)
        if argv[1] == "run":
            return result(argv, next(capacities))
        return result(argv, "reclaimed")

    policy = controlled_policy()
    decision = ensure_capacity(
        CachePaths(repository_root=tmp_path, policy=policy),
        policy,
        "default",
        reason="test pressure",
        runner=runner,
    )

    assert decision.pruned and decision.violations == ()
    assert issued[1] == (
        "docker",
        "builder",
        "prune",
        "--force",
        "--filter",
        "until=72h",
        "--keep-storage",
        "80B",
    )


def test_capacity_failure_names_the_floor(tmp_path: Path) -> None:
    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        return result(argv, "200 200 0")

    policy = controlled_policy()
    decision = ensure_capacity(
        CachePaths(repository_root=tmp_path, policy=policy),
        policy,
        "default",
        reason="test pressure",
        runner=runner,
    )

    assert any("requires 10 free bytes" in violation for violation in decision.violations)
