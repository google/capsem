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
        if argv[1:3] == ("container", "ls") or argv[1:3] == ("image", "ls"):
            return result(argv)
        if argv[1:3] == ("system", "df"):
            return result(
                argv,
                '{"Type":"Build Cache","TotalCount":"1","Active":"0",'
                '"Size":"50B","Reclaimable":"40B"}',
            )
        if argv[1:3] == ("image", "rm"):
            return result(argv, "reclaimed")
        if argv[1:3] == ("builder", "prune"):
            return result(argv, "reclaimed")
        raise AssertionError(argv)

    policy = controlled_policy()
    decision = ensure_capacity(
        CachePaths(repository_root=tmp_path, policy=policy),
        policy,
        "default",
        reason="test pressure",
        runner=runner,
    )

    assert decision.pruned and decision.violations == ()
    assert issued[-2] == (
        "docker",
        "builder",
        "prune",
        "--force",
        "--all",
        "--reserved-space",
        "25B",
    )


def test_pressure_retires_superseded_images_before_buildkit(tmp_path: Path) -> None:
    issued = []
    capacities = iter(("200 200 0", "200 190 10"))

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        issued.append(argv)
        if argv[1] == "run":
            return result(argv, next(capacities))
        if argv[1:3] == ("system", "df"):
            return result(
                argv,
                '{"Type":"Build Cache","TotalCount":"1","Active":"0",'
                '"Size":"50B","Reclaimable":"40B"}',
            )
        if argv[1:3] == ("container", "ls"):
            return result(argv)
        if argv[1:3] == ("image", "ls"):
            return result(
                argv,
                '{"ID":"sha256:old","Repository":"capsem-tool"}\n'
                '{"ID":"sha256:new","Repository":"capsem-tool"}',
            )
        if argv[1:3] == ("image", "inspect"):
            return result(
                argv,
                r'sha256:old\t2026-01-01T00:00:00Z\t10\t["capsem-tool:old"]'
                "\n"
                r'sha256:new\t2026-01-02T00:00:00Z\t10\t["capsem-tool:new"]',
            )
        if argv[1:3] == ("image", "rm"):
            return result(argv, "reclaimed")
        raise AssertionError(argv)

    policy = controlled_policy()
    decision = ensure_capacity(
        CachePaths(repository_root=tmp_path, policy=policy),
        policy,
        "default",
        reason="test pressure",
        runner=runner,
    )

    assert decision.pruned and decision.violations == ()
    assert any(argv[1:3] == ("image", "rm") and argv[-1] == "capsem-tool:old" for argv in issued)
    assert not any(argv[1:3] == ("builder", "prune") for argv in issued)


def test_capacity_failure_names_the_floor(tmp_path: Path) -> None:
    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        if argv[1:3] == ("system", "df"):
            return result(
                argv,
                '{"Type":"Build Cache","TotalCount":"1","Active":"0",'
                '"Size":"5B","Reclaimable":"5B"}',
            )
        if argv[1:3] == ("container", "ls") or argv[1:3] == ("image", "ls"):
            return result(argv)
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
