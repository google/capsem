"""Container acceptance: explicit fixture acquisition, then hermetic VM proof."""

from . import pytestsuite, runtimeprepare
from .actions import Script
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .testmodules import InWorkspace


def prefetch(config: GateConfig) -> Step:
    settings = config.functional.kingslanding
    return step(
        "prefetch",
        Script(
            config,
            settings.fixture_script,
            "--output",
            settings.fixture_dir,
            "--platform",
            config.host_arch().docker_platform,
            outside_sandbox=True,
        ),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.DOCKER, Needs.NETWORK}),
        speed=Speed.SLOW,
    )


def suite(config: GateConfig, *, profile: str, benchmark: bool = True) -> pytestsuite.Suite:
    settings = config.functional.kingslanding
    return pytestsuite.Suite(
        label=f"pytest.kingslanding.{profile}",
        paths=(settings.suite_path,),
        ignores=() if benchmark else (settings.benchmark_path,),
        profile=profile,
        contends=(config.exclusive("workspace_binaries"), config.exclusive("apple_vz")),
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
            else (runtimeprepare.prepare(plan, self._config, after=()).ready,)
        )
        phase = plan.phase("kingslanding")
        fixture = phase.add(prefetch(self._config), after=ready)
        phase.add(
            suite(self._config, profile=self._config.suites.pytest.base_profile).as_step(
                self._config
            ),
            after=(fixture,),
        )
        return plan
