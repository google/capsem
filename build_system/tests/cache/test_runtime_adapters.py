"""Native runtime adapters inventory only explicitly owned resources."""

from pathlib import Path

from capsem_builder.cache import dockeradapter, tartadapter
from capsem_builder.cache.runtimemodels import (
    DockerRuntimePolicy,
    ResourceKind,
    RuntimeCommandResult,
    TartRuntimePolicy,
)


def command(argv: tuple[str, ...], stdout: str = "", returncode: int = 0) -> RuntimeCommandResult:
    return RuntimeCommandResult(
        argv=argv,
        returncode=returncode,
        stdout=stdout,
        stderr="" if returncode == 0 else "unavailable",
        duration_ms=1,
    )


def docker_policy() -> DockerRuntimePolicy:
    return DockerRuntimePolicy(
        kind="docker",
        command="docker",
        timeout_seconds=30,
        mutation_timeout_seconds=600,
        receipt_stage="receipts",
        log_stage="logs",
        image_prefixes=("capsem-",),
        container_prefixes=("capsem-",),
        build_cache_owned=True,
        maximum_age_hours=72,
        keep_image_generations=2,
        build_cache_keep_bytes=80_000_000_000,
    )


def test_docker_inventory_reconciles_native_categories_and_owned_resources() -> None:
    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        if argv[1:3] == ("system", "df"):
            return command(
                argv,
                '\n'.join(
                    (
                        '{"Type":"Images","TotalCount":"2","Active":"0",'
                        '"Size":"12GB","Reclaimable":"4GB (33%)"}',
                        '{"Type":"Build Cache","TotalCount":"8","Active":"0",'
                        '"Size":"90GB","Reclaimable":"30GB"}',
                    )
                ),
            )
        if argv[1:3] == ("container", "ls"):
            return command(
                argv,
                '{"ID":"container-1","Names":"capsem-old","Image":"capsem-tool:one",'
                '"State":"exited","Size":"3MB (virtual 1GB)",'
                '"CreatedAt":"2026-08-01 00:00:00 +0000 UTC"}',
            )
        if argv[1:3] == ("image", "ls"):
            return command(
                argv,
                '\n'.join(
                    (
                        '{"ID":"sha256:owned","Repository":"capsem-tool"}',
                        '{"ID":"sha256:foreign","Repository":"ubuntu"}',
                    )
                ),
            )
        if argv[1:3] == ("image", "inspect"):
            return command(
                argv,
                'sha256:owned\\t2026-08-02T00:00:00Z\\t6000000000'
                '\\t["capsem-tool:one"]',
            )
        raise AssertionError(argv)

    report = dockeradapter.inventory("docker", docker_policy(), runner=runner, now_ns=1)

    assert report.available is True
    assert report.native_bytes == 102_000_000_000
    assert report.owned_bytes == 96_003_000_000
    assert [resource.kind for resource in report.resources] == [
        ResourceKind.IMAGE,
        ResourceKind.CONTAINER,
        ResourceKind.BUILD_CACHE,
    ]
    assert report.resources[0].protected is True
    assert report.resources[0].owned is True


def test_docker_unavailable_is_typed_without_guessing_empty_state() -> None:
    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        return command(argv, returncode=127)

    report = dockeradapter.inventory("docker", docker_policy(), runner=runner, now_ns=1)

    assert report.available is False
    assert report.error == "unavailable"
    assert report.resources == ()


def test_tart_inventory_preserves_foreign_and_running_vms(tmp_path: Path) -> None:
    policy = TartRuntimePolicy(
        kind="tart",
        command="tart",
        timeout_seconds=30,
        mutation_timeout_seconds=60,
        receipt_stage="receipts",
        log_stage="logs",
        vm_prefixes=("capsem-glowup-",),
        base_images=("base@sha256:" + "a" * 64,),
        home=str(tmp_path),
    )

    def runner(argv: tuple[str, ...], _timeout: int) -> RuntimeCommandResult:
        return command(
            argv,
            '[{"Name":"capsem-glowup-old","Source":"local","Running":false,"Size":2},'
            '{"Name":"capsem-glowup-live","Source":"local","Running":true,"Size":3},'
            '{"Name":"personal","Source":"local","Running":false,"Size":99}]',
        )

    report = tartadapter.inventory("tart", policy, runner=runner, now_ns=1)

    assert [resource.identity for resource in report.resources] == [
        "capsem-glowup-live",
        "capsem-glowup-old",
        "personal",
    ]
    assert report.resources[0].protected is True
    assert report.resources[2].owned is False
    assert report.owned_bytes == 5 * 1024**3
