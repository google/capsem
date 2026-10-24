"""An explicit test-only artifact for the existing functional VM owner."""

from . import hostbuild, hostpackage
from .actions import Run, Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .phase import Phase
from .qualification import Qualification


def prepare(
    phase: Phase, config: GateConfig, qualification: Qualification, *, after: tuple[Step, ...]
) -> Step:
    settings = config.functional.native_file_content
    source = phase.add(
        step(
            "native-file-content.source",
            Script(config, settings.script, "begin"),
            produces=(config.path(settings.source_receipt),),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=after,
    )
    ready = source
    if (
        not qualification.pulled
        and config.path(qualification.bin_dir).absolute()
        == config.path(config.modules.default_bin_dir).absolute()
    ):
        built = hostbuild.add(
            phase,
            config,
            label="native-file-content.host",
            after=(source,),
            json_log=config.path(settings.host_log),
        )
        ready = phase.add(
            hostpackage.sign_step(config, label="native-file-content.sign"), after=(built,)
        )
    compiled = phase.add(
        step(
            "native-file-content.compile",
            Run(
                settings.cargo_command,
                log=config.path(settings.cargo_log),
                timeout_seconds=settings.build_timeout_seconds,
            ),
            contends=(config.exclusive("workspace_binaries"),),
            produces=(config.path(settings.cargo_log),),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.SLOW,
        ),
        after=(ready,),
    )
    package = () if qualification.package is None else ("--package", qualification.package)
    return phase.add(
        step(
            "native-file-content.assemble",
            Script(
                config, settings.script, "assemble", "--bin-dir", qualification.bin_dir, *package
            ),
            contends=(config.exclusive("workspace_binaries"),),
            produces=(config.path(settings.selection),),
            kind=Kind.PACKAGE,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
        after=(compiled,),
    )
