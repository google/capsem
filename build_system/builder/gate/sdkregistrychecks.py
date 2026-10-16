"""Registry archive construction and cohort validation in the shared fast gate."""

from __future__ import annotations

from .actions import Run
from .config import GateConfig
from .execution import Kind, Speed, Step, step
from .plan import Plan
from .pythonenv import uv_run


def rust_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_rust
    generated = plan.phase("fast.sdk.rust").add(step(
        "generate", Run(uv_run(config, "python", "-m", "capsem_builder.sdkgen", "--check",
                               "--specification", settings.specification, "--rust-source", settings.source)),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after)
    packaged = plan.phase("fast.sdk.rust").add(step(
        "package", Run(uv_run(config, "python", "-m", "capsem_builder.release.tools.package_rust_sdk")),
        kind=Kind.PACKAGE, speed=Speed.FAST,
        contends=(config.exclusive("workspace_binaries"),),
    ), after=(generated,))
    return generated, packaged


def package_cohort(config: GateConfig) -> Step:
    """Complete qualification checks the same archive metadata publication uses."""
    return step(
        "fast.sdk.packages.validate",
        Run(uv_run(config, "python", "-m", "capsem_builder.release.tools.client_package_archives")),
        kind=Kind.PACKAGE, speed=Speed.FAST,
    )

