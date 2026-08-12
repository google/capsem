"""The input-keyed, network-denied image used by install qualification."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from enum import StrEnum

from . import config as gate_config
from . import hostimage, installbuilder, snapshot
from .actions import Call
from .command import GateCommand
from .config import GateConfig
from .docker import Docker
from .errors import GateError
from .execution import Step, step
from .imageidentity import exact_image_id, exact_image_reference, require_input_key
from .invocation import ConsoleMode
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .outside import Outside
from .plan import Plan
from .proc import Runner
from .storage import Storage

INPUT_KEY_LABEL = "org.capsem.install-image.input-key"


class InstallImageStep(StrEnum):
    CAPACITY = "install.capacity"
    MATERIALIZE = "install.materialize"
    BUILD = "install.image-build"
    SMOKE = "install.image-smoke"


def _step_label(value: InstallImageStep) -> str:
    """Force lifecycle labels through the closed enum at Ty check time."""
    if not isinstance(value, InstallImageStep):
        raise TypeError("install lifecycle labels must be InstallImageStep enum members")
    return value.value


@dataclass(frozen=True)
class InstallImageIdentity:
    input_key: str
    image_id: str
    image_reference: str


def source_image_tag(
    config: GateConfig,
    *,
    helper_id: str,
    source_digest: str | None = None,
) -> str:
    """Key the derived image by exact helper, source bytes, host, and policy."""
    digest = hashlib.blake2b(digest_size=16)
    for value in (
        helper_id,
        source_digest or snapshot.digest(config.root, config),
        config.host_arch().name,
        config.host_arch().docker_platform,
        config.install.builder.source_build_network,
        config.install.smoke_network,
        config.install.runtime_network,
        config.install.source_cli,
    ):
        digest.update(value.encode())
        digest.update(b"\0")
    return config.install.builder.source_tag_template.format(digest=digest.hexdigest())


def _smoke(runner: Runner, config: GateConfig, *, image: str) -> None:
    settings = config.install
    names = config.environment.install
    python = settings.venv_python
    command = (
        "set -e; sudo -n true; cd /src; "
        "source /src/scripts/doctor-linux.sh; linux_musl_toolchain_available; "
        f"{python} -m pytest --version; "
        f"{settings.source_cli} version; "
        f"{python} -m pytest -p no:cacheprovider -q tests/test_materialize_config_http.py"
    )
    passed = Docker(runner).probe(
        image=image,
        command=["bash", "-lc", command],
        network=settings.smoke_network,
        user=settings.guest_user.name,
        env={
            names.project_environment: settings.venv,
            names.test_output_root: settings.test_output_root,
        },
    )
    if not passed:
        raise GateError(
            f"{settings.dockerfile} produced a sealed image that cannot run "
            "the install gate's pinned tools"
        )


def _source_repository(config: GateConfig) -> str:
    return config.install.builder.source_tag_template.split(":", 1)[0]


def build_source_image(
    runner: Runner,
    config: GateConfig,
    *,
    identity: installbuilder.InstallBuilderIdentity,
) -> InstallImageIdentity:
    """Build current source on the exact helper with BuildKit networking denied."""
    docker = Docker(runner)
    helper = installbuilder.require_local_image(runner, config, expected=identity)
    platform = config.host_arch().docker_platform
    tag = source_image_tag(config, helper_id=identity.image_id)
    docker.build(
        tag=tag,
        dockerfile=str(config.path(config.install.dockerfile)),
        context=str(config.root),
        args=[
            f"BASE={helper}",
            f"INPUT_IDENTITY={tag}",
            f"FRESH_CLI={config.install.source_cli}",
        ],
        platform=platform,
        network=config.install.builder.source_build_network,
        console=ConsoleMode.LOG_ONLY,
    )
    installbuilder.require_local_image(runner, config, expected=identity)
    require_input_key(
        docker,
        tag,
        label=INPUT_KEY_LABEL,
        subject="install qualification image",
    )
    image_id = exact_image_id(
        docker,
        tag,
        platform=platform,
        subject="install qualification image",
    )
    reference = exact_image_reference(
        docker,
        tag,
        platform=platform,
        expected_id=image_id,
        subject="install qualification image",
    )
    found = InstallImageIdentity(tag, image_id, reference)
    runner.note(
        f"Install image: input key {tag}; exact image {image_id}; build reference {reference}"
    )
    Storage(runner).reclaim(_source_repository(config), keep=tag)
    return found


def prepare(runner: Runner) -> InstallImageIdentity:
    """Materialize dependencies once, build sealed source once, and smoke once."""
    config = gate_config.for_root(runner.root)
    helper = installbuilder.materialize(runner, config)
    image = build_source_image(runner, config, identity=helper)
    _smoke(runner, config, image=image.input_key)
    return image


def require_local_image(runner: Runner, config: GateConfig) -> str:
    """Return the runnable input-key tag only after binding it to the exact ID."""
    docker = Docker(runner)
    helper = installbuilder.require_current(runner, config)
    platform = config.host_arch().docker_platform
    tag = source_image_tag(config, helper_id=helper.image_id)
    if not docker.image_exists(tag, platform=platform):
        raise GateError(f"install qualification image {tag} is missing")
    require_input_key(
        docker,
        tag,
        label=INPUT_KEY_LABEL,
        subject="install qualification image",
    )
    image_id = exact_image_id(
        docker,
        tag,
        platform=platform,
        subject="install qualification image",
    )
    runner.note(f"Using install qualification image {tag}, exact child {image_id}")
    return tag


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Expose the sole egress edge and both sealed phases in the gate graph."""
    built = hostimage.fragment(plan, config, after=after)

    capacity = plan.shared(
        step(
            _step_label(InstallImageStep.CAPACITY),
            Call(
                "reserve disk for the install helper and exact source image",
                lambda context: Storage(context.runner).ensure_space("install-preflight"),
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="the storage policy measures and reserves Docker capacity",
                    effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(built, *after),
    )

    def materialize(context) -> None:
        identity = installbuilder.materialize(context.runner, context.config)
        context.journal.note(
            f"install helper: input key {identity.input_key}; exact image {identity.image_id}; "
            f"build reference {identity.image_reference}"
        )

    materialized = plan.shared(
        step(
            _step_label(InstallImageStep.MATERIALIZE),
            Outside(
                Call(
                    "materialize locked install qualification dependencies",
                    materialize,
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="the exact host-builder child and helper input key resolve at run time",
                        effects=machine_effects(
                            Effect.PROCESS,
                            Effect.FILESYSTEM,
                            Effect.NETWORK,
                            Effect.HOST_STATE,
                        ),
                    ),
                )
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(capacity,),
    )

    def build(context) -> None:
        helper = installbuilder.require_current(context.runner, context.config)
        identity = build_source_image(context.runner, context.config, identity=helper)
        context.journal.note(
            f"install image: input key {identity.input_key}; exact image {identity.image_id}; "
            f"build reference {identity.image_reference}"
        )

    image = plan.shared(
        step(
            _step_label(InstallImageStep.BUILD),
            Call(
                "build the network-denied install qualification image",
                build,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="helper identity and source digest resolve at run time",
                    effects=machine_effects(Effect.PROCESS, Effect.FILESYSTEM, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(materialized,),
    )

    def smoke(context) -> None:
        exact = require_local_image(context.runner, context.config)
        _smoke(context.runner, context.config, image=exact)
        context.journal.note(f"sealed install image smoke passed: exact image {exact}")

    return plan.shared(
        step(
            _step_label(InstallImageStep.SMOKE),
            Call(
                "smoke the exact install image with networking denied",
                smoke,
                justification=CallJustification(
                    kind=OpaqueKind.RUNTIME_DERIVED,
                    reason="the exact source image ID is revalidated immediately before smoke",
                    effects=machine_effects(Effect.PROCESS, Effect.HOST_STATE),
                ),
            ),
            contends=(config.exclusive("docker_daemon"),),
        ),
        after=(image,),
    )


class InstallImageCommand(
    GateCommand,
    name="install-image",
    help="materialize and smoke the sealed install qualification image",
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan
