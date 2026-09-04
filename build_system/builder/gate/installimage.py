"""The input-keyed, network-denied image used by install qualification."""

from __future__ import annotations

import hashlib
import time
from enum import StrEnum
from pathlib import Path

from . import config as gate_config
from . import imagecache, installbuilder, installreceipt, sourcecapture
from .actions import Action
from .cachecontrol import CacheControl
from .config import GateConfig
from .context import Context
from .docker import Docker
from .errors import GateError
from .filesystem import remove
from .imageidentity import (
    exact_image_id,
    exact_image_reference,
    require_input_key,
)
from .invocation import ConsoleMode
from .proc import Runner

INPUT_KEY_LABEL = "org.capsem.install-image.input-key"


class InstallImageStep(StrEnum):
    CACHE = "install.cache"
    MATERIALIZE = "install.materialize"
    BUILD = "install.image-build"
    SMOKE = "install.image-smoke"


def _step_label(value: InstallImageStep) -> str:
    """Force lifecycle labels through the closed enum at Ty check time."""
    if not isinstance(value, InstallImageStep):
        raise TypeError("install lifecycle labels must be InstallImageStep enum members")
    return value.value


InstallImageIdentity = installreceipt.InstallImageIdentity


def source_image_tag(
    config: GateConfig,
    *,
    helper_id: str,
    source: sourcecapture.SourceSnapshot,
) -> str:
    """Key the derived image by exact helper, source bytes, host, and policy."""
    if not isinstance(source, sourcecapture.SourceSnapshot):
        raise TypeError("source must be a SourceSnapshot")
    digest = hashlib.blake2b(digest_size=16)
    for value in (
        helper_id,
        source.digest,
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
        "source /src/build_system/scripts/doctor/doctor-linux.sh; linux_musl_toolchain_available; "
        f"{python} -m pytest -c build_system/pyproject.toml --rootdir . --version; "
        f"{settings.source_cli} version; "
        f"{python} -m pytest -c build_system/pyproject.toml --rootdir . -p no:cacheprovider -q "
        "tests/test_materialize_config_http.py"
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


def _receipt_path(config: GateConfig) -> Path:
    relative = Path(config.install.builder.source_identity_file)
    if relative.is_absolute() or not relative.parts or ".." in relative.parts:
        raise GateError("install source_identity_file must stay beneath the checkout")
    return config.path(str(relative))


def _write_receipt(config: GateConfig, identity: InstallImageIdentity) -> None:
    installreceipt.write(_receipt_path(config), identity)


def _read_receipt(config: GateConfig) -> InstallImageIdentity:
    return installreceipt.read(_receipt_path(config))


def build_source_image(
    runner: Runner,
    config: GateConfig,
    *,
    identity: installbuilder.InstallBuilderIdentity,
    source: sourcecapture.SourceSnapshot,
) -> InstallImageIdentity:
    """Build the recorded source on the exact helper with networking denied."""
    sourcecapture.require_snapshot(config, source)
    try:
        receipt = installreceipt.validate(
            runner,
            config,
            path=_receipt_path(config),
            receipt=_read_receipt(config),
            helper=identity,
            source=source,
            tag=source_image_tag(config, helper_id=identity.image_id, source=source),
            resource=_source_repository(config),
            input_key_label=INPUT_KEY_LABEL,
            touch=True,
        )
    except GateError as error:
        runner.note(f"Install image cache miss: {error}; rebuilding")
    else:
        runner.note(
            f"Install image cache hit is mandatory: reusing {receipt.input_key}, "
            f"exact child {receipt.image_id}"
        )
        return receipt
    remove(_receipt_path(config))
    docker = Docker(runner)
    helper = installbuilder.require_local_image(runner, config, expected=identity)
    platform = config.host_arch().docker_platform
    tag = source_image_tag(config, helper_id=identity.image_id, source=source)
    docker.build(
        tag=tag,
        dockerfile=str(source.root / config.install.dockerfile),
        context=str(source.root),
        args=[
            f"BASE={helper}",
            f"INPUT_IDENTITY={tag}",
            f"FRESH_CLI={config.install.source_cli}",
        ],
        platform=platform,
        network=config.install.builder.source_build_network,
        console=ConsoleMode.LOG_ONLY,
    )
    sourcecapture.require_snapshot(config, source)
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
    runtime_digest = installreceipt.digest(docker.runtime_identity())
    image_size = docker.image_size(tag)
    policy = CacheControl(runner).image_policy(_source_repository(config))
    if image_size > policy.max_size_bytes:
        raise GateError(
            f"install qualification image is {image_size} bytes, above the configured "
            f"{policy.max_size_bytes}-byte cache bound"
        )
    now = time.time()
    found = InstallImageIdentity(
        schema_version=imagecache.RECEIPT_SCHEMA,
        input_key=tag,
        input_digest=installreceipt.digest(
            tag,
            identity.input_key,
            identity.image_id,
            source.digest,
            runtime_digest,
        ),
        image_id=image_id,
        image_reference=reference,
        helper_input_key=identity.input_key,
        helper_image_id=identity.image_id,
        source_digest=source.digest,
        runtime_digest=runtime_digest,
        platform=platform,
        image_size_bytes=image_size,
        created_at=now,
        last_used_at=now,
    )
    runner.note(
        f"Install image: input key {tag}; exact image {image_id}; build reference {reference}"
    )
    _write_receipt(config, found)
    CacheControl(runner).reclaim(
        _source_repository(config),
        keep=tag,
        protect=imagecache.protected_tags(config, _source_repository(config), field="input_key"),
    )
    return found


def prepare(runner: Runner) -> InstallImageIdentity:
    """Materialize dependencies once, build sealed source once, and smoke once."""
    config = gate_config.for_root(runner.root)
    source = sourcecapture.require_recorded(config)
    helper = installbuilder.materialize(runner, config)
    image = build_source_image(runner, config, identity=helper, source=source)
    _smoke(runner, config, image=image.input_key)
    return image


def require_local_image(runner: Runner, config: GateConfig) -> str:
    """Return the runnable input-key tag only after binding it to the exact ID."""
    receipt = _read_receipt(config)
    source = sourcecapture.require_recorded(config)
    helper = installbuilder.require_current(runner, config)
    found = installreceipt.validate(
        runner,
        config,
        path=_receipt_path(config),
        receipt=receipt,
        helper=helper,
        source=source,
        tag=source_image_tag(config, helper_id=helper.image_id, source=source),
        resource=_source_repository(config),
        input_key_label=INPUT_KEY_LABEL,
        touch=True,
    )
    runner.note(
        f"Using install qualification image {found.input_key}, exact child {found.image_id}"
    )
    return found.input_key


class RequireInstallImage(Action, name="require-install-image"):
    """Resume check for the persisted exact source-image product."""

    def render(self) -> str:
        return "require the exact receipted install qualification image"

    def perform(self, context: Context) -> None:
        require_local_image(context.runner, context.config)
