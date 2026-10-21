"""Materialize and optionally publish every private build-helper image."""

from __future__ import annotations

from ..image import assettools, guestbuilder
from ..image.config import load_guest_config
from ..image.docker import asset_dependency_tag
from ..privateimages import PrivateImage
from . import assetdependencies, hostimage, imagebases, installbuilder, linuxrust, packagebuilder
from .actions import Action
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .docker import Docker
from .execution import Kind, Needs, Speed, step
from .plan import Plan
from .privateimagecache import publish


def inventory(config: GateConfig, docker: Docker) -> tuple[PrivateImage, ...]:
    """Every network-open helper generation consumed by this host's gates."""
    host_arch = config.host_arch()
    platform = host_arch.docker_platform
    host_key = hostimage.input_key(config)
    linux_tag = linuxrust.base_tag(config, docker)
    install_tag = installbuilder.image_tag(config, docker)
    build = imagebases.build_config(config)
    guest = load_guest_config(config.path(config.imagebuild.source_config))

    images = [
        PrivateImage(
            config.hostimage.tag,
            platform,
            host_key,
            hostimage.INPUT_KEY_LABEL,
            host_key,
        ),
        PrivateImage(
            linux_tag,
            platform,
            linux_tag.rsplit(":", 1)[1],
            linuxrust.INPUT_KEY_LABEL,
            linux_tag,
        ),
        PrivateImage(
            install_tag,
            platform,
            install_tag.rsplit(":", 1)[1],
            installbuilder.INPUT_KEY_LABEL,
            install_tag,
        ),
    ]
    for target in config.architectures.values():
        package_tag = packagebuilder.image_tag(config, target, docker)
        images.append(
            PrivateImage(
                package_tag,
                platform,
                package_tag.rsplit(":", 1)[1],
                packagebuilder.INPUT_KEY_LABEL,
                package_tag,
            )
        )
        guest_tag = guestbuilder.image_tag(build, target.name, config.root)
        guest_platform = guestbuilder.environment(build, target.name).docker_platform
        images.append(
            PrivateImage(
                guest_tag,
                guest_platform,
                guest_tag.rsplit(":", 1)[1],
                guestbuilder.INPUT_KEY_LABEL,
                guest_tag,
            )
        )
    tools_tag = assettools.image_tag(build, host_arch.name, config.root)
    images.append(
        PrivateImage(
            tools_tag,
            platform,
            tools_tag.rsplit(":", 1)[1],
            assettools.INPUT_KEY_LABEL,
            tools_tag,
        )
    )
    for template in config.imagebuild.lane_templates:
        dependency_tag = asset_dependency_tag(guest, host_arch.name, template)
        images.append(
            PrivateImage(
                dependency_tag,
                platform,
                dependency_tag.rsplit(":", 1)[1],
                "org.capsem.asset-dependencies.input-key",
                dependency_tag,
            )
        )
    return tuple(images)


class MaterializeProductHelpers(Action, name="private-product-helpers"):
    """Build or reuse install and package dependency helpers."""

    def render(self) -> str:
        return "materialize private install and package dependency helpers"

    def perform(self, context: Context) -> None:
        installbuilder.materialize(context.runner, context.config)
        for target in context.config.architectures.values():
            packagebuilder.materialize(context.runner, context.config, target)


class PublishPrivateImages(Action, name="private-images-publish"):
    """Publish the complete locally validated helper inventory."""

    def render(self) -> str:
        return "publish immutable private image generations"

    def perform(self, context: Context) -> None:
        config = context.config
        publish(
            context.runner,
            config,
            inventory(config, Docker(context.runner)),
            config.path(config.private_images.manifest),
        )


class PrivateImagesCommand(
    GateCommand,
    name="private-images",
    help="materialize every private build helper and optionally publish it",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("--publish", action="store_true")

    def plan(self) -> Plan:
        config = self._config
        plan = Plan(self.name)
        host = plan.shared(hostimage.image(config))
        linux = plan.add(
            step(
                "private.linux-rust-base",
                linuxrust.WarmBase(),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.COMPILE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(host,),
        )
        products = plan.add(
            step(
                "private.product-helpers",
                MaterializeProductHelpers(),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(host,),
        )
        names = tuple(config.architectures)
        bases = plan.add(
            step(
                "private.asset-bases",
                imagebases.Prefetch((config.host_arch().name,), rust_names=names, asset_tools=True),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            )
        )
        guest_builders = plan.add(
            step(
                "private.guest-builders",
                imagebases.MaterializeRustBuilders(names),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(bases,),
        )
        tools = plan.add(
            step(
                "private.asset-tools",
                imagebases.MaterializeAssetTools(),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(guest_builders,),
        )
        dependencies = plan.add(
            assetdependencies.dependency_step(
                config,
                (config.host_arch().name,),
                label="private.asset-dependencies",
            ),
            after=(tools,),
        )
        if self._args.publish:
            plan.add(
                step(
                    "private.publish",
                    PublishPrivateImages(),
                    contends=(config.exclusive("docker_daemon"),),
                    kind=Kind.PACKAGE,
                    needs=frozenset({Needs.DOCKER, Needs.DISK, Needs.NETWORK}),
                    speed=Speed.SLOW,
                ),
                after=(linux, products, dependencies),
            )
        return plan
