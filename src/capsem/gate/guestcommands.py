"""Entering a guest, and running one command in a throwaway one.

Split out of `release.py` for the same reason as the development surfaces:
neither has anything to do with cutting a release.

`exec` is the command whose argument crosses into a guest, which is why the
recipe hands it `{{quote(CMD)}}` and it keeps the payload as one exact string.
"""

from __future__ import annotations

from .actions import Run
from .command import GateCommand
from .execution import Kind, Needs, Speed, step
from .plan import Plan


class ShellCommand(GateCommand, name="shell", help="start the service and enter a temporary VM"):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "shell",
                Run([self._config.logs.cli, "shell"]),
                contends=(self._config.exclusive("apple_vz"),),
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
                speed=Speed.SLOW,
            )
        )
        return plan


class ExecCommand(GateCommand, name="exec", help="run one command in a fresh temporary VM"):
    """One string, carried to the guest without a host shell ever seeing it.

    `guest_command`, not `command`: the latter is the subparser's own slot, and
    a positional named that overwrote the subcommand name -- registry lookup
    then indexed a dict with a list and the public command could not dispatch.

    One positional rather than `nargs="+"`, because the payload is a command
    line for the guest, and rejoining a list is where its quoting is lost.
    `capsem run`, not `capsem exec`: the Rust CLI's `exec` executes in an
    *existing* session and takes one, so the payload would have been consumed
    as a session name. `run` is the one-shot fresh session this documents.
    """

    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("guest_command")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "exec",
                Run([self._config.logs.cli, "run", self._args.guest_command]),
                contends=(self._config.exclusive("apple_vz"),),
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.VM, Needs.KVM, Needs.DISK}),
                speed=Speed.SLOW,
            )
        )
        return plan
