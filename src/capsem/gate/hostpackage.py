"""Signing, the host SBOM, the desktop bundle, and reading logs back.

Four small recipes that shared nothing but their smallness. Each carried one
decision worth keeping and a shell body worth losing: which binaries get
codesigned, how many packages a release cohort should contain, that the
frontend bundle must exist before cargo reads it, and where a preserved
failure went.
"""

from __future__ import annotations

import json

from . import host
from .actions import Action, Run, Script
from .command import GateCommand
from .config import GateConfig
from .context import Context
from .errors import GateError
from .execution import step
from .plan import Plan


def sign_step(config: GateConfig, *, label: str = "sign"):
    """Codesign the host binaries, or nothing at all off macOS.

    A step rather than `Run(["just", "_sign"])` at each call site: the two test
    modules that need signed binaries run inside a held machine lock, so
    dispatching there was a child waiting for the lock its parent held.

    The label is a parameter because two fragments in one composed plan both
    need signing, at different points -- once after the coverage build and
    again before the VM suites -- and a plan cannot hold two steps of one name.
    """
    settings = config.signing
    return step(
        label,
        *[
            Run(
                [
                    "codesign",
                    "--sign",
                    "-",
                    "--entitlements",
                    settings.entitlements,
                    "--force",
                    binary,
                ]
            )
            for binary in settings.binaries
        ],
        contends=(config.exclusive("workspace_binaries"),),
        # The bytes a later step executes. Declared here, at the fragment, so
        # every command that composes signing inherits the claim rather than
        # each composer remembering to repeat it.
        produces=tuple(config.path(binary) for binary in settings.binaries),
    )


def sbom_step(config: GateConfig):
    """Generate the host package SBOM and check it describes something."""
    return step("host-sbom", _GenerateSbom(), _ValidateSbom())


class SignCommand(GateCommand, name="sign", help="codesign the host binaries for VM tests"):
    """Apple Virtualization.framework refuses an unsigned caller, so this is a
    precondition for every VM test rather than a packaging nicety."""

    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if not host.on_macos():
            return plan
        plan.add(sign_step(self._config))
        return plan


class HostSbomCommand(
    GateCommand, name="host-sbom", help="generate and validate the host package SBOM"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(sbom_step(self._config))
        return plan


def _artifacts(config: GateConfig) -> list[str]:
    """Every publishable host package for the current version.

    Exactly two `.deb`s, one per architecture. Fewer means a build did not
    happen; more means an older version is still in `dist/` and the SBOM would
    describe a cohort nobody is shipping.
    """
    from .versions import workspace_version

    settings = config.sbom
    version = workspace_version(config.root)
    debs = sorted(config.root.glob(settings.dist_glob.format(version=version)))
    if len(debs) != settings.expected_debs:
        raise GateError(
            f"expected {settings.expected_debs} current-version Linux packages, "
            f"found {len(debs)}: {[p.name for p in debs]}"
        )

    found = [str(path) for path in debs]
    if host.on_macos():
        package = config.path(settings.macos_package.format(version=version))
        if not package.is_file() or package.stat().st_size == 0:
            raise GateError(f"missing or empty macOS package: {package}")
        found.append(str(package))
    return found


class _GenerateSbom(Action, name="generate-sbom"):
    def render(self) -> str:
        return "generate the host package SBOM from every publishable artifact"

    def perform(self, context: Context) -> None:
        settings = context.config.sbom
        Script(settings.script, "--output", settings.output, *_artifacts(context.config)).perform(
            context
        )


class _ValidateSbom(Action, name="validate-sbom"):
    """A document that parses is not a document that describes anything."""

    def render(self) -> str:
        return "check the SBOM is SPDX and lists the packaged executables"

    def perform(self, context: Context) -> None:
        settings = context.config.sbom
        document = json.loads(context.config.path(settings.output).read_text(encoding="utf-8"))
        if document.get("spdxVersion") != settings.spdx_version:
            raise GateError(f"host SBOM is not {settings.spdx_version}")
        if not document.get("files"):
            raise GateError("host SBOM contains no packaged executables")
        context.journal.note(f"host SBOM validated: {len(document['files'])} files")


class BuildUiCommand(
    GateCommand, name="build-ui", help="build the frontend bundle and the desktop app"
):
    """The bundle first, always.

    `capsem-app` embeds `frontend/dist` at compile time through
    `tauri::generate_context!`, so a cargo build that runs first embeds
    whatever was there before -- which is why a rebuilt frontend appeared to
    change nothing.
    """

    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("profile", nargs="?", default="debug")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.frontend
        profile = self._args.profile
        if profile not in settings.profiles:
            raise GateError(
                f"unknown build profile {profile!r}; expected one of {', '.join(settings.profiles)}"
            )

        bundle = plan.add(
            step("frontend", Run(["bash", settings.build_script, settings.build_target]))
        )
        argv = ["cargo", "build", "-p", settings.app_crate]
        if profile != settings.profiles[0]:
            argv.append(f"--{profile}")
        plan.add(step(f"app.{profile}", Run(argv)), after=(bundle,))
        return plan


class LogsCommand(
    GateCommand, name="logs", help="tail the service log, or show a preserved failure"
):
    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("target", nargs="?", default="", help="a sandbox id, or `failure`")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.logs
        target = self._args.target

        if target == "failure":
            plan.add(step("failure", _ShowPreservedFailure()))
        elif target:
            plan.add(step("sandbox", Run([settings.cli, "logs", target])))
        else:
            plan.add(step("service", Run(["tail", "-f", str(host.home() / settings.service_log)])))
        return plan


class _ShowPreservedFailure(Action, name="show-preserved-failure"):
    def render(self) -> str:
        return "list the most recently preserved failure evidence"

    def perform(self, context: Context) -> None:
        root = context.config.path(context.config.logs.failure_root)
        preserved = (
            sorted((entry for entry in root.iterdir() if entry.is_dir()), reverse=True)
            if root.is_dir()
            else []
        )
        if not preserved:
            raise GateError(f"no preserved test failure under {root}")

        latest = preserved[0]
        context.journal.note(str(latest))
        for path in sorted(latest.rglob("*")):
            if path.is_file():
                context.journal.note(f"  {path}")
