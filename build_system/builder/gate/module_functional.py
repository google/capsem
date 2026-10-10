"""The module that runs the suites needing a booted VM."""

from __future__ import annotations

from dataclasses import dataclass, replace

from . import (
    audits,
    hostpackage,
    kingslanding,
    pytestsuite,
    runtimeprepare,
    sdkchecks,
    toolchain,
    vmproofs,
)
from .command import GateCommand
from .config import GateConfig
from .content import RuntimeContent
from .contentcheck import ContentComplete
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .qualification import Qualification
from .testmodules import InWorkspace


class FunctionalModule(
    InWorkspace,
    GateCommand,
    name="test-functional",
    help="every VM-owned suite, against the one runtime",
):
    """The slowest thing the gate does.

    There is one runtime, so there is one lane: the broad proof, then the
    VM-owned suites that share the machine, then the measurements alone.

    What may not overlap is declared rather than achieved by placement. In
    shell these ran in sequence below a `wait` and stayed correct only because
    nobody added a job underneath.
    """

    uses_qualification = True
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self.qualification.pulled:
            functional(plan, self._config, qualification=self.qualification)
        else:
            permission = self.rebuild_permission
            prepared = runtimeprepare.prepare(plan, self._config, permission=permission)
            functional(
                plan,
                self._config,
                qualification=self.qualification,
                after=(prepared.ready,),
                signed=prepared.ready,
            )
        return plan


def functional(
    plan: Plan,
    config: GateConfig,
    *,
    qualification: Qualification,
    after: tuple[Step, ...] = (),
    isolated_assets: bool = False,
    staged: RuntimeContent | None = None,
    generated: Step | None = None,
    node: Step | None = None,
    signed: Step | None = None,
    source_contracts_proved: bool = False,
) -> Step:
    """Every VM-owned suite, against the runtime under test.

    `staged` is absolute, and only a release lane passes it. That lane stages
    its cohort into the workspace and then qualifies from a private prefix
    which carries none of it, so a checkout-relative answer points at a
    directory nothing ever wrote.
    """
    phase = plan.phase("functional")
    content = _content_selector(config, staged=staged, isolated=isolated_assets)

    # That the content the suites will boot is complete is a run-time question,
    # asked once, before any suite: a plan may not depend on build output.
    checked = (
        (phase.add(_content_step(content), after=after),)
        if content.root is not None
        else after
    )

    # This module owns its prerequisites, the same way `module_contracts` had
    # to learn to. The broad suite renders the release site from fixtures with
    # `pnpm --dir build_system/release_site run build`, and `node_modules` is gitignored --
    # so a local run worked on whatever an earlier phase had installed, and the
    # release lane, whose prefix carries only tracked files, died on a missing
    # Astro. Idempotent, and the `node_modules` exclusive keeps the two
    # installs in a candidate plan from overlapping.
    #
    # Handed over when a composed run already installed it, exactly as
    # `generated` is below. Not `plan.shared`: two lanes want this step at two
    # different points in the order, so a single shared node inherits both
    # sets of edges and closes a cycle -- which is the reordering
    # `_already_issuing` documents as the reason dedup lives at the call site
    # rather than inside `Plan.add`.
    sdk_node = node if node is not None else phase.add(
        toolchain.node(config, config.functional.node_workspaces), after=checked,
    )
    # SDK producers need their toolchain, not VM content. In a composed plan
    # they already belong to the fast phase; adding the later runtime-content
    # edge to those shared producers would close a dependency cycle.
    sdk_ready = sdkchecks.braavos(plan, phase, config, after=(sdk_node,))
    python_consumer = phase.add(
        sdkchecks.python_package_prewarm(config),
        after=(sdk_ready[0], *checked),
    )
    prepared: tuple[Step, ...] = (sdk_node, *checked, *sdk_ready, python_consumer)
    # The generated mock is gitignored, so it is never part of the source a run
    # is given, and the broad suite checks it for staleness. Made here when
    # this module runs alone, and handed over when a composed run has already
    # made it. `ready` stays in the chain either way: depending on the
    # handed-over step *instead* dropped this module's own ordering.
    settled: tuple[Step, ...] = (
        (generated, *prepared)
        if generated is not None
        else (phase.add(audits.generated_settings(config), after=prepared),)
    )

    # A release lane was handed signed binaries; signing them again would
    # replace the bytes the manifest selected with locally built ones.
    first: tuple = settled
    if not qualification.pulled:
        first = (
            (signed, *settled)
            if signed is not None
            else (phase.add(hostpackage.sign_step(config), after=settled),)
        )

    fixture = phase.add(kingslanding.prefetch(config), after=first)
    # The suites that can share the machine, in order: they claim Apple VZ and
    # the workspace binaries shared, and the xdist suite claims the VM fleet
    # alone so its four VMs never overlap another fleet.
    current = phase.add(
        content.suite(
            pytestsuite.broad(config, source_contracts_proved=source_contracts_proved)
        ).as_step(config),
        after=(fixture,),
    )
    for owned in (
        kingslanding.suite(config, benchmark=False),
        kingslanding.greyjoy_suite(config),
        pytestsuite.host_snapshot(config),
    ):
        current = phase.add(content.suite(owned).as_step(config), after=(current,))
    current = phase.add(vmproofs.injection(config, **content.proof_arguments()), after=(current,))
    current = phase.add(
        vmproofs.integration(config, **content.proof_arguments()), after=(current,)
    )
    # Measurements hold Apple VZ alone, so they run last, one at a time.
    for suite in (
        pytestsuite.timing(config),
        kingslanding.benchmark_suite(config),
        pytestsuite.benchmark(config),
    ):
        current = phase.add(content.suite(suite).as_step(config), after=(current,))
    return current


@dataclass(frozen=True)
class _Content:
    """The content the suites and VM proofs are pointed at, if not the checkout's."""

    config: GateConfig
    root: RuntimeContent | None = None

    def _assets(self) -> str | None:
        return None if self.root is None else str(self.root.assets)

    def suite(self, suite: pytestsuite.Suite) -> pytestsuite.Suite:
        assets = self._assets()
        return suite if assets is None else replace(suite, assets_dir=assets)

    def proof_arguments(self) -> dict[str, str | None]:
        return {"assets": self._assets()}


def _content_selector(
    config: GateConfig, *, staged: RuntimeContent | None, isolated: bool
) -> _Content:
    # A release lane's cohort is one staged asset tree. Without this the suites
    # inherit no content selection at all and fall back to the checkout --
    # which, inside the prefix, is the one place the lane never staged anything.
    if staged is not None:
        return _Content(config, staged)
    if isolated:
        return _Content(config, RuntimeContent.built(config))
    return _Content(config)


def _content_step(content: _Content) -> Step:
    assert content.root is not None
    return step(
        "content",
        ContentComplete(content.root),
        kind=Kind.UNIT_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )
