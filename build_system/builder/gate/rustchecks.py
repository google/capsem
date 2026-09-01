"""What proves the Rust workspace, once there is something built to prove it.

Split out of `testmodules`, which was over the three-hundred-line ceiling the
gate holds itself to. The seam is a real one rather than a convenience: these
are the checks that need a *compiled* workspace, which is why none of them is
in the fast phase and why they share one environment and one exclusive.

Clippy is deliberately not here. It only checks, so it answers in the fast
phase with the other cheap gates. The three-way division between clippy,
Nextest and doctests is held by
`tests/citadel/test_rust_check_coverage.py`.
"""

from __future__ import annotations

from . import toolchain
from .actions import Run, Script
from .config import GateConfig
from .execution import SATURATES, Kind, Needs, Speed, Step, step


def environment(config: GateConfig) -> dict[str, str]:
    """What both Rust runs need, including Nextest's timeout policy.

    The profile arrives as an environment variable because `--profile` on the
    command line is cargo-llvm-cov's *build* profile; passing `ci` there would
    silently select a Cargo profile of that name, or fail, and in neither case
    apply the `slow-timeout` that is the point of using Nextest at all.
    """
    return {
        **toolchain.ort_environment(config, toolchain.OrtConsumer.STATIC),
        config.modules.rust_test_profile_variable: config.modules.rust_test_profile,
    }


def coverage(config: GateConfig) -> Step:
    """Every native test target, under Nextest, against the coverage floor."""
    settings = config.modules
    return step(
        "rust-coverage",
        Run(
            [*settings.rust_coverage, *settings.rust_coverage_floors],
            env=environment(config),
        ),
        Script(
            config,
            settings.rust_coverage_ratchet,
            "--report",
            settings.rust_coverage_report,
            "--crate-root",
            settings.rust_coverage_crate_root,
        ),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.UNIT_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
        concurrency=SATURATES,
    )


def doctests(config: GateConfig) -> Step:
    """The targets Nextest does not run, and never will.

    A separate step rather than a flag on the one above, for two reasons.
    Nextest genuinely cannot run doctests -- `rustinventory` models them as a
    distinct target set for that reason -- so this is the only thing executing
    them once the runner has been swapped. And its own label means the timing
    summary can say what they cost, instead of hiding them inside coverage.
    """
    return step(
        "rust-doctests",
        Run(list(config.modules.rust_doctests), env=environment(config)),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.UNIT_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )
