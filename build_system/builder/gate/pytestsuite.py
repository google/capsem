"""One spelling of "run pytest", used sixteen times.

`_test-candidate-run` invoked pytest eleven times and `smoke` five more, each
assembling its own flags. They agreed by hand: the same `--tb=short`, the same
`--maxfail` budget, the same four `--ignore` directories, the same
`CAPSEM_REQUIRE_ARTIFACTS=1`, spelled out again each time. Sixteen copies of an
agreement is sixteen opportunities for one of them to be slightly different,
and no way to notice which.

The interesting part is what *cannot* share a machine, and each case has a
specific reason recorded beside it in `[execution.exclusives]`:

  host snapshot tests    production has one service and one service-scoped
                         save/restore lock; an xdist worker per service does
                         not reproduce that

  benchmarks             two files launching VMs at once measure each other
                         rather than Capsem

In shell those were achieved by placement -- run after the `wait`, and hope
nobody adds a job below. Here they are declared, and the plan will not overlap
them whatever order the steps are written in.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum

from .actions import Run
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .harnessschema import Exclusive
from .pythonenv import pytest


class CoverageMode(StrEnum):
    """How one pytest cohort participates in a composed coverage proof."""

    NONE = "none"
    SINGLE = "single"
    SEED = "seed"
    APPEND = "append"
    FINISH = "finish"

    def __bool__(self) -> bool:
        return self is not CoverageMode.NONE


@dataclass(frozen=True)
class Suite:
    """One pytest invocation, described rather than spelled out."""

    label: str
    paths: tuple[str, ...] = ()
    markers: str = ""
    ignores: tuple[str, ...] = ()
    ignore_globs: tuple[str, ...] = ()
    deselect: str = ""
    parallel: bool = False
    coverage: CoverageMode = CoverageMode.NONE
    stop_at_first_failure: bool = True
    profile: str = ""
    assets_dir: str = ""
    profiles_dir: str = ""
    project: str = ""
    require_artifacts: bool = True
    contends: tuple[Exclusive, ...] = field(default_factory=tuple)

    def argv(self, config: GateConfig) -> list[str]:
        settings = config.suites.pytest
        project = self.project or settings.build_system_project
        if project != settings.build_system_project:
            raise ValueError(f"unknown Python test project {project!r}")
        argv = pytest(config, *self.paths, *settings.base_flags)

        if self.stop_at_first_failure:
            argv.append(settings.stop_at_first)
        if self.parallel:
            argv += [
                "-n",
                str(settings.parallel_workers),
                f"--dist={settings.parallel_distribution}",
            ]
        coverage_flags = {
            CoverageMode.NONE: (),
            CoverageMode.SINGLE: settings.coverage_flags,
            CoverageMode.SEED: settings.coverage_seed_flags,
            CoverageMode.APPEND: settings.coverage_append_flags,
            CoverageMode.FINISH: settings.coverage_finish_flags,
        }
        argv += coverage_flags[self.coverage]
        if self.markers:
            argv += ["-m", self.markers]
        if self.deselect:
            argv += ["-k", self.deselect]

        argv += [f"--ignore={path}" for path in self.ignores]
        argv += [f"--ignore-glob={pattern}" for pattern in self.ignore_globs]
        return argv

    def environment(self, config: GateConfig) -> dict[str, str]:
        settings = config.suites.pytest
        env: dict[str, str] = {}
        if self.require_artifacts:
            # Fails closed before collection, rather than passing vacuously
            # against a tree whose assets were never built.
            env[settings.require_artifacts] = "1"
        if self.profile:
            env[settings.profile_variable] = self.profile
        if self.assets_dir or self.profiles_dir:
            if not self.assets_dir or not self.profiles_dir:
                raise ValueError("a pytest content selection requires both assets and profiles")
            env.update(
                config.environment.content(
                    assets=self.assets_dir,
                    profiles=self.profiles_dir,
                )
            )
        return env

    def as_step(self, config: GateConfig) -> Step:
        return step(
            self.label,
            Run(self.argv(config), env=self.environment(config)),
            contends=self.contends,
            kind=Kind.UNIT_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.SLOW,
            concurrency=(config.suites.pytest.parallel_workers if self.parallel else 1),
        )


def citadel(config: GateConfig) -> Suite:
    """The guards recording architectural mistakes that must not be repeated.

    In the fast phase rather than the broad suite. Every one reads source and
    asserts on it -- no artifact, no VM, no daemon -- so `require_artifacts` is
    off and the whole suite answers in seconds.

    It was reached only through the broad suite's `root`, which carries
    `require_artifacts` and runs after the whole asset build. That meant a
    guard whose own docstring says it exists to "fail before a hidden route
    cache, direct SQLite open, or compatibility fallback can ship green"
    reported the violation once the VMs were already up.

    `stop_at_first_failure` is off deliberately: these are independent guards
    over unrelated boundaries, and knowing all of what regressed is worth more
    than saving a second.
    """
    return Suite(
        label="citadel",
        paths=(config.suites.pytest.citadel,),
        parallel=True,
        stop_at_first_failure=False,
        require_artifacts=False,
    )


def collection(config: GateConfig) -> Step:
    """Strictly collect both Python test roots in one interpreter."""
    settings = config.suites.pytest
    return step(
        "pytest.collection",
        Run(
            pytest(
                config,
                settings.root,
                settings.build_system_root,
                *settings.collection_flags,
            )
        ),
        kind=Kind.STATIC_TEST,
        speed=Speed.FAST,
    )


# ---------------------------------------------------------------------------
# The suites the functional module is made of
# ---------------------------------------------------------------------------


def broad(
    config: GateConfig,
    *,
    profile: str,
    source_contracts_proved: bool = False,
) -> Suite:
    """Everything that can share a machine, four VMs at a time.

    The dogfooding canary. `--dist=loadfile` keeps per-file fixtures on one
    worker, which matters because the fixtures build VMs.
    """
    settings = config.suites.pytest
    proven_paths = config.suites.source_contract if source_contracts_proved else ()
    proven_globs = config.modules.contract_globs if source_contracts_proved else ()
    return Suite(
        label=f"pytest.broad.{profile}",
        paths=(settings.root,),
        markers="not serial",
        ignores=(*settings.host_snapshot_serial, *settings.broad_ignores, *proven_paths),
        ignore_globs=proven_globs,
        parallel=True,
        coverage=(CoverageMode.FINISH if source_contracts_proved else CoverageMode.SINGLE),
        profile=profile,
        contends=(config.exclusive("workspace_binaries"),),
    )


def host_snapshot(config: GateConfig, *, profile: str) -> Suite:
    """The suites that need to be the only service on the machine."""
    settings = config.suites.pytest
    return Suite(
        label=f"pytest.host-snapshot.{profile}",
        paths=settings.host_snapshot_serial,
        markers="not serial",
        profile=profile,
        contends=(config.exclusive("host_service"),),
    )


def timing(config: GateConfig, *, profile: str) -> Suite:
    """Timing probes, alone, so their numbers mean something."""
    settings = config.suites.pytest
    return Suite(
        label=f"pytest.timing.{profile}",
        paths=settings.serial_paths,
        markers="serial",
        deselect=settings.benchmark_deselect,
        stop_at_first_failure=False,
        profile=profile,
        contends=(config.exclusive("apple_vz"),),
    )


def benchmark(config: GateConfig, *, profile: str) -> Suite:
    """The recorded baseline, which is the whole point of running alone."""
    settings = config.suites.pytest
    return Suite(
        label=f"pytest.benchmark.{profile}",
        paths=(settings.benchmark_baseline,),
        stop_at_first_failure=False,
        profile=profile,
        contends=(config.exclusive("apple_vz"),),
    )


def compatibility(config: GateConfig, *, profile: str) -> Suite:
    """Every VM-owned suite again, for a second selected profile.

    The compatibility axis, not a reduced substitute: the broad suite proves
    the source and runtime contracts once, and this proves the VM behaviour
    holds for each remaining profile the channel selects.
    """
    settings = config.suites.pytest
    return Suite(
        label=f"pytest.compatibility.{profile}",
        paths=(settings.root,),
        markers="(integration or mcp or e2e) and not serial",
        ignores=(
            *settings.host_snapshot_serial,
            *config.suites.source_contract,
            *settings.broad_ignores,
        ),
        ignore_globs=config.modules.contract_globs,
        parallel=True,
        profile=profile,
        contends=(config.exclusive("workspace_binaries"),),
    )
