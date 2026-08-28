"""Build the complete local macOS product and install that exact package."""

from __future__ import annotations

from . import assetplan, host
from .actions import Call, Run
from .command import GateCommand
from .content import ProfileContent
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .opacity import CallJustification, OpaqueKind, machine_effects
from .plan import Plan
from .versions import workspace_version


class LocalInstallCommand(
    GateCommand,
    name="local-install",
    help="build the complete local macOS product and install it for hands-on testing",
):
    """The developer-machine path, distinct from Docker/Tart qualification."""

    exclusive = True

    def plan(self) -> Plan:
        if not host.on_macos():
            raise GateError("local-install currently requires macOS")

        plan = Plan(self.name)
        config = self._config
        content = ProfileContent.isolated(
            config,
            config.path(config.assets.test_root) / config.suites.pytest.base_profile,
        )
        version = workspace_version(config.root)
        package = config.path(config.sbom.macos_package.format(version=version))

        assets = assetplan.fragment(plan, config)
        verified = plan.add(
            step(
                "local-install.content",
                Call(
                    "verify the exact local assets and materialized configuration pair",
                    lambda _context: content.require_complete(config),
                    justification=CallJustification(
                        kind=OpaqueKind.PURE_INSPECTION,
                        reason="the native package consumes one inseparable local content pair",
                        effects=machine_effects(),
                    ),
                ),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ),
            after=(assets,),
        )
        built = plan.add(
            step(
                "local-install.package",
                Run(
                    [
                        "bash",
                        config.install.local_macos_package_script,
                        "--version",
                        version,
                        "--manifest-url",
                        (content.assets / config.install.manifest_name).resolve().as_uri(),
                        "--assets-dir",
                        str(content.assets),
                        "--config-root",
                        str(content.config),
                    ]
                ),
                contends=(config.exclusive("workspace_binaries"),),
                produces=(package,),
                kind=Kind.PACKAGE,
                needs=frozenset({Needs.DISK, Needs.SIGNING}),
                speed=Speed.SLOW,
            ),
            after=(verified,),
        )
        plan.add(
            step(
                "local-install.install",
                Run(
                    [
                        *config.install.local_macos_installer,
                        str(package),
                        config.install.local_macos_target,
                    ],
                    outside_sandbox=True,
                ),
                kind=Kind.E2E,
                needs=frozenset({Needs.DISK, Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(built,),
        )
        return plan
