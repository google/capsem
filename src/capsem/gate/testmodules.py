"""The modules `just test` is made of, and the ones provable from source.

`_test-candidate-run` was 366 lines because six modules each re-solved the same
problems inside one `bash` body, selected by a `CAPSEM_TEST_MODULE` environment
variable and a `module_enabled` function. A module was a region of a file
between two `if` statements, which is why running one in isolation meant
setting an environment variable and hoping, and why nothing could say what one
would do without doing it.

Each is a command now. It declares the workspace it needs and the graph of
steps it contains, and both are answerable for free.

`vmmodules` holds the three that cannot start against a bare checkout, because
they need built artifacts or a VM. The seam is what a module needs before it
can begin.
"""

from __future__ import annotations

from . import (
    audits,
    host,
    hostimage,
    hostpackage,
    installimage,
    pytestsuite,
    storage,
    toolchain,
)
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import step
from .fileactions import RequireFile
from .lifecycle import Resource
from .plan import Plan
from .workspace import Workspace


class InWorkspace:
    """Runs against an isolated `CAPSEM_HOME`, never the developer's.

    A mixin rather than a base command, because a base would have to register
    itself as a runnable name and there is nothing to run.
    """

    exclusive = True

    _config: GateConfig
    """Supplied by `GateCommand`, declared here so the mixin type-checks."""

    def resources(self) -> tuple[Resource, ...]:
        return (Workspace(self._config),)


class FastModule(
    InWorkspace,
    GateCommand,
    name="test-fast",
    help="the checks that fail in minutes rather than in forty",
):
    """Cheap, independent, and the most common failure class.

    Everything here is free to overlap except one edge: clippy reads
    `frontend/dist`, which `capsem-app` embeds at compile time, so the frontend
    build must finish first. The shell expressed that as a conditional which
    skipped clippy entirely when the frontend failed -- losing the clippy
    result on exactly the runs where the most had changed.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config

        # The environment first: everything below runs through uv or pnpm,
        # and a gate that assumes the lockfile is already installed is a
        # gate that works on the machine it was written on.
        python = plan.add(toolchain.sync(config))
        node = plan.add(toolchain.node(config), after=(python,))
        rust = plan.add(toolchain.rust(config), after=(python,))

        # Nothing is worth starting against a file that will not parse.
        syntax = plan.add(audits.source_syntax(config), after=(python,))

        for check in audits.all_of(config):
            plan.add(check, after=(syntax,))
        plan.add(audits.lint(config), after=(syntax,))

        surfaces = [
            plan.add(surface, after=(syntax, node)) for surface in audits.web_surfaces(config)
        ]
        plan.add(
            audits.clippy(config),
            after=(audits.blocking_surface(config, surfaces), rust),
        )
        return plan


class StaticModule(
    InWorkspace,
    GateCommand,
    name="test-static",
    help="the source-build proofs, before anything boots a VM",
):
    """What can be proved from source, in the order the proofs depend on.

    The install-harness preflight comes first for a blunt reason: proving the
    clean container can launch its runner takes a minute, and discovering it
    cannot after the Rust coverage run wastes twenty.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.modules

        preflight = installimage.fragment(plan, config)
        plan.add(storagerelease(config, "install-preflight"), after=(preflight,))

        # `_pack-initrd` already built the host architecture; this proves the
        # other one compiles against musl, so a cross-arch regression surfaces
        # before the Docker cross-compile rather than an hour later.
        agents = plan.add(step("guest-agents", Run(settings.guest_agent_build)))
        binaries = plan.add(_guest_binaries_present(config), after=(agents,))
        plan.add(
            pytestsuite.Suite(
                label="guest-binary-contracts",
                paths=settings.guest_binary_tests,
                stop_at_first_failure=False,
                require_artifacts=False,
            ).as_step(config),
            after=(binaries,),
        )

        if host.on_macos():
            # Native Linux runs exercise these cfg branches directly. A Mac
            # host has to run the same checked-in Linux runner in Docker, or
            # Linux-only regressions stay out of the local gate entirely.
            linux = hostimage.linux_rust(plan, config)
            plan.add(storagerelease(config, "linux-rust-builder"), after=(linux,))

        coverage = plan.add(
            step(
                "rust-coverage",
                Run([*settings.rust_coverage, settings.rust_coverage_floor]),
                contends=(config.exclusive("workspace_binaries"),),
            ),
            after=(agents,),
        )
        plan.add(hostpackage.sign_step(config), after=(coverage,))
        return plan


class ReleaseContractsModule(
    GateCommand,
    name="test-release-contracts",
    help="the release and composition contracts, without artifacts",
):
    """Cheap, and deliberately excludes what needs a built tree.

    The build-chain suites that require artifacts are the artifacts module's
    job. Running them here would either fail on a fresh checkout or pass
    vacuously, and both are worse than not running them.
    """

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.modules

        # The glob is expanded here. `bash` expanded it before pytest ever saw
        # it; pytest does not expand path arguments itself, so passing the
        # pattern through collects nothing and the module passes vacuously.
        contracts = sorted(
            str(path.relative_to(config.root))
            for path in config.root.glob(settings.contract_glob)
        )
        if not contracts:
            raise GateError(f"no contract tests matched {settings.contract_glob}")

        plan.add(
            pytestsuite.Suite(
                label="contracts",
                paths=(
                    *settings.release_suites,
                    *contracts,
                    *config.suites.source_contract,
                ),
                ignores=settings.build_chain_artifact_tests,
                stop_at_first_failure=False,
                require_artifacts=False,
            ).as_step(config)
        )
        return plan


def _guest_binaries_present(config: GateConfig):
    """Every guest binary the host architecture should have produced."""
    root = config.path(config.modules.guest_binary_root) / config.host_arch().name
    return step(
        "guest-binaries",
        *[RequireFile(root / name) for name in config.modules.guest_binaries],
    )


def storagerelease(config: GateConfig, phase: str):
    """Hand back the storage a finished rail was holding.

    The one spelling lives in `storage`; this is the name the modules use.
    """
    return storage.release_step(config, phase)
