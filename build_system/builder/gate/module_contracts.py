"""The release and composition contracts, as their own module.

Split out of `testmodules` when that file crossed the 300-line ceiling. The
seam is the one the ceiling kept pointing at: `testmodules` holds the modules
that prove *source* -- the cheap checks and the source-build proofs -- and this
holds the one that proves the release and composition surface. They change for
different reasons, and this one has a lifecycle rule none of the others share.
"""

from __future__ import annotations

from . import audits, pytestsuite, sandbox, toolchain
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Step
from .plan import Plan


class ReleaseContractsModule(
    GateCommand,
    name="test-release-contracts",
    help="the release and composition contracts, without artifacts",
):
    """No prebuilt artifacts required; some contracts build their own fixtures.

    The build-chain suites that require artifacts are the artifacts module's
    job. Running them here would either fail on a fresh checkout or pass
    vacuously, and both are worse than not running them.

    The standalone command deliberately does not take the machine lock: its
    kernel-lease regressions need to acquire that lock in child probes. Holding
    it here stalled the command for 27 minutes against 4 minutes 41 directly.

    The collectors include native Cargo fixtures that can mutate shared build
    outputs. Standalone builds need the bounded-command lease; inside a
    candidate they reuse its outer lease. Both steps therefore declare
    `workspace_binaries` alongside `astro_build` and `node_modules`, so the
    composed scheduler excludes other workspace users while the tests run.
    """

    sandboxed = sandbox.ENFORCE
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        release_contracts(plan, self._config)
        return plan



def _once(*paths: str) -> tuple[str, ...]:
    """The same file named by a glob and by `source_contract` is still one file.

    pytest collects a path given twice twice, and reports the duplicate as a
    passing test, so the count goes up while the coverage does not.
    """
    seen: dict[str, None] = {}
    for path in paths:
        seen.setdefault(path, None)
    return tuple(seen)


def release_contracts(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    node: Step | None = None,
    generated: Step | None = None,
    seed_coverage: bool = False,
) -> Step:
    """The release and composition contracts, without artifacts."""
    phase = plan.phase("contracts")
    settings = config.modules

    # The glob is expanded here. `bash` expanded it before pytest ever saw it;
    # pytest does not expand path arguments itself, so passing the pattern
    # through collects nothing and the module passes vacuously.
    contracts = sorted(
        {
            str(path.relative_to(config.root))
            for pattern in settings.contract_globs
            for path in config.root.glob(pattern)
        }
    )
    if not contracts:
        raise GateError(f"no contract tests matched {settings.contract_globs}")

    # This module owns its prerequisites, which AGENTS.md requires of every
    # one of them and this one did not do. `test_local_multichannel_dist_contract`
    # runs `pnpm --dir build_system/release_site run build:channel`, and `node_modules` is
    # gitignored -- so on a warm machine an earlier build had installed it and
    # on a clean one the suite died with `sh: astro: command not found`.
    #
    # A composed candidate hands over its already-complete node step. The
    # standalone command still owns this prerequisite, so independence does
    # not require paying for the same workspace install twice in one plan.
    installed = node or phase.add(toolchain.node(config), after=after)
    generated = generated or phase.add(
        audits.generated_settings(config), after=(*after, installed)
    )

    build_root = config.suites.pytest.build_system_root.rstrip("/") + "/"
    root_contracts = tuple(
        path
        for path in _once(
            *settings.release_suites,
            *contracts,
            *config.suites.source_contract,
        )
        if not path.startswith(build_root)
    )
    prerequisites = (*after, installed) if node is not None else (installed,)
    root = phase.add(
        pytestsuite.Suite(
            label="release",
            paths=root_contracts,
            ignores=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
            require_artifacts=False,
            # Ten minutes seventeen in one process: over half of what
            # `fast-test` costs, and more than the whole lane's budget allows.
            # Contracts read workflows, plans and configuration; some also
            # build native or release-site fixtures. `--dist=loadfile` keeps
            # each file's fixtures on one worker, while contention excludes
            # other gate steps that use the same shared outputs.
            parallel=True,
            coverage=(
                pytestsuite.CoverageMode.SEED
                if seed_coverage
                else pytestsuite.CoverageMode.NONE
            ),
            # Release-site and native Cargo fixtures mutate shared outputs.
            # A candidate's outer lease excludes other processes; these claims
            # also exclude competing steps in this same plan.
            contends=(
                config.exclusive("astro_build"),
                config.exclusive("node_modules"),
                config.exclusive("workspace_binaries"),
            ),
        ).as_step(config),
        after=prerequisites,
    )
    return phase.add(
        pytestsuite.Suite(
            label="build-system",
            paths=(config.suites.pytest.build_system_root,),
            project=config.suites.pytest.build_system_project,
            stop_at_first_failure=False,
            require_artifacts=False,
            parallel=True,
            coverage=(
                pytestsuite.CoverageMode.APPEND
                if seed_coverage
                else pytestsuite.CoverageMode.NONE
            ),
            contends=(
                config.exclusive("astro_build"),
                config.exclusive("node_modules"),
                config.exclusive("workspace_binaries"),
            ),
        ).as_step(config),
        after=(*prerequisites, root, generated),
    )
