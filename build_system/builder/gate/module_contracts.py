"""The release and composition contracts, as their own module.

Split out of `testmodules` when that file crossed the 300-line ceiling. The
seam is the one the ceiling kept pointing at: `testmodules` holds the modules
that prove *source* -- the cheap checks and the source-build proofs -- and this
holds the one that proves the release and composition surface. They change for
different reasons, and this one has a lifecycle rule none of the others share.
"""

from __future__ import annotations

from . import pytestsuite, sandbox, toolchain
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
    """Cheap, and deliberately excludes what needs a built tree.

    The build-chain suites that require artifacts are the artifacts module's
    job. Running them here would either fail on a fresh checkout or pass
    vacuously, and both are worse than not running them.

    Deliberately **not** exclusive, unlike every other module -- and the one
    place the "anything that writes takes the machine lock" rule is wrong.
    This command runs the source-contract suite, which contains the gate's own
    tests: holding the machine lock while running tests that exercise the gate
    stalls the command outright. Measured at 27 minutes wall for 27 seconds of
    CPU, against 4 minutes 41 for the identical selection run directly.

    Its real contention is `astro_build` and `node_modules`, declared on the
    step where the graph can act on them. A command that *runs tests* is not a
    command that mutates shared artifacts, whatever the general rule says.
    """

    sandboxed = sandbox.ENFORCE

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


def release_contracts(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
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
    # runs `pnpm --dir release-site run build:channel`, and `node_modules` is
    # gitignored -- so on a warm machine an earlier build had installed it and
    # on a clean one the suite died with `sh: astro: command not found`.
    #
    # In the candidate plan this makes `pnpm install --frozen-lockfile` run
    # twice; it is idempotent, both steps declare the `node_modules` exclusive
    # so they cannot overlap, and the second is a no-op against a warm tree.
    # That is a cheaper answer than a module whose independence depends on
    # having been run after another one.
    installed = phase.add(toolchain.node(config), after=after)

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
    root = phase.add(
        pytestsuite.Suite(
            label="release",
            paths=root_contracts,
            ignores=settings.build_chain_artifact_tests,
            stop_at_first_failure=False,
            require_artifacts=False,
            # Ten minutes seventeen in one process: over half of what
            # `fast-test` costs, and more than the whole lane's budget allows.
            # These are source-level contracts -- they read workflows, plans and
            # configuration -- and `--dist=loadfile` keeps each file's tests on
            # one worker, so the fixtures that build at fixed paths stay inside
            # the file that builds them.
            parallel=True,
            # It builds release-site fixtures at fixed paths and installs the
            # workspace's node modules. As its own command a machine lock made
            # that safe by accident; in a shared plan it has to be declared.
            contends=(
                config.exclusive("astro_build"),
                config.exclusive("node_modules"),
            ),
        ).as_step(config),
        after=(installed,),
    )
    return phase.add(
        pytestsuite.Suite(
            label="build-system",
            paths=(config.suites.pytest.build_system_root,),
            project=config.suites.pytest.build_system_project,
            stop_at_first_failure=False,
            require_artifacts=False,
            parallel=True,
            contends=(
                config.exclusive("astro_build"),
                config.exclusive("node_modules"),
            ),
        ).as_step(config),
        after=(installed, root),
    )
