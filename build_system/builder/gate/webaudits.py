"""Web build checks and the Rust work they order in the fast graph."""

from __future__ import annotations

from . import clippyrun, toolchain
from .actions import Run
from .config import GateConfig
from .execution import SATURATES, Kind, Needs, Speed, Step, step


def surfaces(config: GateConfig) -> list[Step]:
    """One step per web surface, serializing only Astro bundle owners."""
    policy = config.websurfaces
    return [
        step(
            f"web.{target}",
            Run(["bash", policy.script, target]),
            contends=(config.exclusive("astro_build"),) if target in policy.building else (),
            kind=Kind.COMPILE if target in policy.building else Kind.STATIC_TEST,
            speed=Speed.FAST,
        )
        for target in policy.targets
    ]


def frontend_bundle(config: GateConfig) -> Step:
    """Build the exact bundle Tauri embeds, without rerunning frontend tests."""
    frontend = config.frontend
    return step(
        "web.frontend-bundle",
        Run(["bash", frontend.build_script, frontend.build_target]),
        contends=(config.exclusive("astro_build"), config.exclusive("node_modules")),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
    )


def release_channel(config: GateConfig) -> Step:
    """Build release-channel fixtures under their real Cargo contention claim."""
    return step(
        "web.release-channel",
        Run(["bash", config.websurfaces.script, "release-channel"]),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.E2E,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


def blocking_surface(config: GateConfig, checks: list[Step]) -> Step:
    """Return the configured web surface that must precede Clippy."""
    wanted = f"web.{config.websurfaces.blocks_clippy}"
    return next(candidate for candidate in checks if candidate.label.endswith(wanted))


def _keyed_clippy(config: GateConfig, cargo_args: list[str]) -> Run:
    """`cargo clippy <cargo_args> -- -D warnings`, keyed by this checkout."""
    argv, environment = clippyrun.invocation(
        config.toolchain.clippy_workspace_wrapper, cargo_args, ["-D", "warnings"]
    )
    return Run(argv, env={**toolchain.ort_environment(config, toolchain.OrtConsumer.FAST), **environment})


def clippy(config: GateConfig) -> Step:
    """Run the project-standard all-target Rust lint with warnings denied."""
    return step(
        "clippy",
        _keyed_clippy(config, ["--workspace", "--all-targets"]),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
        concurrency=SATURATES,
    )


def clippy_guest(config: GateConfig) -> Step:
    """Lint the guest feature set with every target, on the host toolchain.

    Nothing here embeds the frontend, so unlike `clippy` this does not wait
    for the bundle: it is a second compile of two crates, not of the workspace.
    """
    packages = [part for package in config.initrd.lint_packages for part in ("-p", package)]
    return step(
        "clippy.guest",
        _keyed_clippy(
            config,
            [
                *packages,
                "--no-default-features",
                "--features",
                ",".join(config.initrd.lint_features),
                "--all-targets",
            ],
        ),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
        concurrency=SATURATES,
    )
