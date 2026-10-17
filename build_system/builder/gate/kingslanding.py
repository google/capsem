"""Container acceptance: explicit fixture acquisition, then hermetic VM proof."""

from . import pytestsuite, runtimeprepare
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .testmodules import InWorkspace

#: The `[functional.<section>]` pins every kingslanding run verifies first.
PINNED_IMAGES = ("debug_image", "reference_image")


def prefetch(config: GateConfig) -> Step:
    """Every pinned image the suites serve: Redis and iperf3, capsem-debug and
    the reference image, whose layouts are verified in the shared cache and
    pulled by digest only when absent."""
    settings = config.functional.kingslanding
    platform = config.host_arch().docker_platform
    return step(
        "prefetch",
        Script(
            config,
            settings.fixture_script,
            "--output",
            settings.fixture_dir,
            "--platform",
            platform,
            outside_sandbox=True,
        ),
        *(
            Script(
                config,
                getattr(config.functional, section).script,
                section,
                "prepare",
                "--platform",
                platform,
                outside_sandbox=True,
            )
            for section in PINNED_IMAGES
        ),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.DOCKER, Needs.NETWORK}),
        speed=Speed.SLOW,
    )


def greyjoy_suite(config: GateConfig) -> pytestsuite.Suite:
    """The chaos suite: the same fixture, its own owner, adversaries only."""
    return pytestsuite.Suite(
        label="pytest.greyjoy",
        paths=(config.functional.greyjoy.suite_path,),
        contends=pytestsuite.sharing(config),
    )


def suite(config: GateConfig, *, benchmark: bool = True) -> pytestsuite.Suite:
    """The acceptance suite; with its measurement files it needs the machine alone."""
    settings = config.functional.kingslanding
    return pytestsuite.Suite(
        label="pytest.kingslanding",
        paths=(settings.suite_path,),
        ignores=() if benchmark else settings.benchmark_paths,
        contends=pytestsuite.measuring(config) if benchmark else pytestsuite.sharing(config),
    )


def benchmark_suite(config: GateConfig) -> pytestsuite.Suite:
    """Kingslanding's measurement files, split out so the rest can share the machine."""
    return pytestsuite.Suite(
        label="pytest.kingslanding-benchmark",
        paths=config.functional.kingslanding.benchmark_paths,
        contends=pytestsuite.measuring(config),
    )


class KingslandingModule(
    InWorkspace,
    GateCommand,
    name="test-kingslanding",
    help="container lifecycle, isolation and Redis publication through real VMs",
):
    uses_qualification = True
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        ready = (
            ()
            if self.qualification.pulled
            else (
                runtimeprepare.prepare(
                    plan, self._config, permission=self.rebuild_permission
                ).ready,
            )
        )
        phase = plan.phase("kingslanding")
        fixture = phase.add(prefetch(self._config), after=ready)
        phase.add(
            suite(self._config).as_step(self._config),
            after=(fixture,),
        )
        return plan


class GreyjoyModule(
    InWorkspace,
    GateCommand,
    name="test-greyjoy",
    help="adversaries against private networks through real VMs: kills, floods, deletions",
):
    uses_qualification = True
    outside_egress = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        ready = (
            ()
            if self.qualification.pulled
            else (
                runtimeprepare.prepare(
                    plan, self._config, permission=self.rebuild_permission
                ).ready,
            )
        )
        phase = plan.phase("greyjoy")
        fixture = phase.add(prefetch(self._config), after=ready)
        phase.add(
            greyjoy_suite(self._config).as_step(self._config),
            after=(fixture,),
        )
        return plan
