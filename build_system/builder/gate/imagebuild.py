"""Build the VM runtime assets through the one config-owned image rail.

There is one runtime. Applications come from OCI images, so the only thing a
build varies is the architecture and the template.
"""

from __future__ import annotations

from . import assetdependencies, crossexec, imagebases, initrd
from .actions import Run
from .assetcondition import missing as missing
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .imagedoctor import doctor
from .plan import Plan

#: Naming the capability sets rather than the whole declaration: `**dict`
#: unpacking would collapse three lines into one and take the type checking
#: with it, which is the thing these attributes exist to keep.
BUILDS = frozenset({Needs.DOCKER, Needs.DISK})
PULLS = frozenset({Needs.DOCKER, Needs.NETWORK})


def build_argv(
    config: GateConfig,
    *,
    arch: str | None,
    template: str,
    output: str | None = None,
) -> list[str]:
    """The one spelling of `capsem-admin image build` for either output rail."""
    settings = config.imagebuild
    if template not in settings.templates:
        raise GateError(
            f"unknown image template {template!r}; expected one of {', '.join(settings.templates)}"
        )

    argv = [
        *settings.admin,
        "--config-root",
        settings.config_root,
        "--output",
        output or settings.output,
        "--template",
        template,
        "--clean",
    ]
    if arch:
        argv += ["--arch", config.arch(arch).name]
    return argv


def build(
    config: GateConfig,
    *,
    arch: str | None,
    template: str,
    output: str | None = None,
) -> Step:
    """One image build. The template is the only thing that varies."""
    label = f"image.{template}" + (f".{arch}" if arch else "")
    return step(
        label,
        Run(build_argv(config, arch=arch, template=template, output=output)),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.PACKAGE,
        needs=BUILDS,
        speed=Speed.SLOW,
    )


class BuildAssetsCommand(
    GateCommand,
    name="build-assets",
    help="build the VM runtime assets for one architecture, or every one",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("arch", nargs="?", help="defaults to every architecture")
        parser.add_argument("--template", default="all", help="kernel, rootfs, or all")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        names = (
            (config.arch(self._args.arch).name,) if self._args.arch else tuple(config.architectures)
        )
        rust_builders = (
            ()
            if self._args.template == "kernel"
            else imagebases.required_rust_builder_names(config, names)
        )
        needs_asset_tools = self._args.template != "kernel"
        bases = plan.add(
            step(
                "base-images",
                imagebases.Prefetch(names, rust_names=rust_builders, asset_tools=needs_asset_tools),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=PULLS,
                speed=Speed.SLOW,
            )
        )
        checked = plan.add(doctor(config), after=(bases,))
        ready = plan.add(
            step(
                "guest-execution",
                crossexec.Require(names),
                contends=(config.exclusive("docker_daemon"),),
                kind=Kind.PACKAGE,
                needs=BUILDS,
                speed=Speed.SLOW,
            ),
            after=(checked,),
        )
        if rust_builders:
            ready = plan.add(
                step(
                    "guest-builders",
                    imagebases.MaterializeRustBuilders(rust_builders),
                    contends=(config.exclusive("docker_daemon"),),
                    carry_checks=(imagebases.RequireRustBuilders(rust_builders),),
                    kind=Kind.PACKAGE,
                    needs=BUILDS,
                    speed=Speed.SLOW,
                ),
                after=(ready,),
            )
        if needs_asset_tools:
            ready = plan.add(
                step(
                    "asset-tools",
                    imagebases.MaterializeAssetTools(),
                    contends=(config.exclusive("docker_daemon"),),
                    carry_checks=(imagebases.RequireAssetTools(),),
                    kind=Kind.PACKAGE,
                    needs=BUILDS,
                    speed=Speed.SLOW,
                ),
                after=(ready,),
            )
        ready = plan.add(
            assetdependencies.request_step(config, names, self._args.template),
            after=(ready,),
        )
        image = plan.add(
            build(config, arch=self._args.arch, template=self._args.template),
            after=(ready,),
        )
        if self._args.template != "kernel":
            assets = config.path(config.imagebuild.output)
            targets = {name: (assets / name / config.artifacts.initrd,) for name in names}
            packed = plan.add(initrd.repack_step(config, targets), after=(image,))
            initrd.finalize(plan, config, assets=assets, arches=names, after=(packed,))
        return plan
