"""`prove-deb`, as a command.

`debproof` runs the proof: start a systemd container, install the exact
package, and check what the installed product does. This is the adapter that
turns it into one step of a plan and registers it on the CLI -- the seam the
module ceiling kept pointing at, and the one that lets the proof be driven
from `packagerail` without a command in the way.
"""

from __future__ import annotations

from pathlib import Path

from .actions import Call, Why
from .command import GateCommand
from .debproof import DebProof
from .execution import step
from .plan import Plan


class ProveDebCommand(
    GateCommand,
    name="prove-deb",
    help="install one exact dist/*.deb in a clean container and prove it",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        """Arguments, not environment.

        The three `CAPSEM_PROOF_*` variables existed to carry these across a
        process boundary the package rail no longer crosses -- it constructs
        `DebProof` directly. What is left is this command's own surface, and a
        surface is better as arguments: `--help` lists them, and a missing one
        is an argparse error rather than a runtime `GateError` deep inside a
        container start.
        """
        parser.add_argument("package", help="the exact .deb to install")
        parser.add_argument("--manifest-url", required=True)
        parser.add_argument("--channel", required=True)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        args = self._args
        plan.add(
            step(
                "prove",
                Call(
                    "install the exact .deb in a clean container",
                    lambda ctx: DebProof(
                        ctx.runner,
                        package=Path(args.package),
                        manifest_url=args.manifest_url,
                        channel=args.channel,
                    ).run(),
                    why=Why.DYNAMIC,
                ),
                contends=(self._config.exclusive("docker_daemon"),),
            )
        )
        return plan
