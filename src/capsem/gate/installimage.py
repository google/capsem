"""The disposable image the install proof runs inside.

Always rebuilt from its checked-in Dockerfile, never merely checked for. Docker
keeps unchanged layers cached, so the rebuild is nearly free -- while asking
whether the tag exists lets a stale local image hide a new CI prerequisite, and
then the gate proves an environment nobody else has.

The smoke check exists because a cached layer can satisfy `docker build` and
still be missing a tool. One cacheless rebuild is attempted when it fails; a
second failure is a defect in the Dockerfile rather than a stale layer, and
saying so is more use than a third attempt.
"""

from __future__ import annotations

from . import config as gate_config
from . import hostimage
from .actions import Call
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Step, step
from .plan import Plan
from .proc import Runner

# The one behaviour here rather than in config: a check, not a value. It
# exercises every tool the install gate depends on -- passwordless sudo,
# cdxgen, the musl toolchain, and a pytest that can actually collect.
SMOKE = (
    "set -e; sudo -n true; cd /src; cdxgen --version; "
    "source /src/scripts/doctor-linux.sh; linux_musl_toolchain_available; "
    "uv run python -m pytest --version; "
    "uv run python -m pytest -p no:cacheprovider -q tests/test_materialize_config_http.py"
)


def _smoke_passes(runner: Runner, settings: gate_config.InstallConfig) -> bool:
    return runner.succeeds(
        [
            "docker", "run", "--rm",
            "-u", "capsem",
            "-e", f"UV_PROJECT_ENVIRONMENT={settings.venv}",
            "-e", f"CAPSEM_TEST_OUTPUT_ROOT={settings.test_output_root}",
            "-v", f"{runner.root}:{settings.mount}:ro",
            settings.image,
            "bash", "-lc", SMOKE,
        ]
    )


def prepare(runner: Runner) -> None:
    """Build the install-test image and prove it can run the gate's tools.

    The builder image it derives from is a separate step, composed by
    `fragment` and ordered ahead of this. It used to be `just
    _build-host-image` from right here -- a recipe with a heading and no body,
    so this whole path failed at runtime and no test crossed the boundary to
    see it.
    """
    settings = gate_config.for_root(runner.root).install
    build = ["docker", "build", "-t", settings.image, "-f", settings.dockerfile, "."]

    runner.run(build)

    if not _smoke_passes(runner, settings):
        runner.note(
            "Install-test image smoke check failed; rebuilding without Docker cache..."
        )
        runner.run([*build[:2], "--no-cache", *build[2:]])
        if not _smoke_passes(runner, settings):
            raise GateError(
                f"{settings.dockerfile} produces an image that cannot run the install "
                "gate's tools even after a cacheless rebuild"
            )

    # The builder image this derives from is *not* released here. It belongs to
    # both package builds, which is why `after-packages` frees it and
    # nothing earlier does -- and this preflight runs first, so a
    # release from here landed 164ms before `cache-ownership` ran that exact
    # image and got exit 125.


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """The install-test image, after the builder image it derives from.

    `after` reaches this step, not the shared image beneath it: groundwork
    several lanes share cannot be sequenced behind any one of them without
    making a cycle out of the next lane that needs it.
    """
    built = hostimage.fragment(plan, config)
    return plan.add(
        step(
            "install-image",
            Call(
                "build the disposable install-test image",
                lambda ctx: prepare(ctx.runner),
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(built, *after),
    )


class InstallImageCommand(
    GateCommand,
    name="install-image",
    help="build and smoke the disposable install-test image",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan
