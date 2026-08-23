"""`just logs`: the service stream, a sandbox's logs, or the last failure.

Its own module because reading a log is not building a package, and because
the thing it got wrong needed room to be stated. It tailed
`~/.capsem/run/service.log` by name. That name is what `telemetry::init` is
*configured* with; the appender rotates daily and writes
`service.<date>.log`, so the file being tailed is either absent or holds only
the raw stderr of a process that died before tracing started. `just logs`
showed almost nothing and looked like a quiet service.

`telemetry.rs` states the rule and gives `log_stream_files` as the one way to
follow it. The same mistake had already been made and fixed twice in the
macOS glow-up and once in the Linux one.
"""

from __future__ import annotations

from . import host
from .actions import Action, Run
from .command import GateCommand
from .context import Context
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .plan import Plan


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
            plan.add(step("failure", _ShowPreservedFailure(),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ))
        elif target:
            plan.add(step("sandbox", Run([settings.cli, "logs", target]),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ))
        else:
            plan.add(step("service", _FollowServiceStream(),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ))
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


class _FollowServiceStream(Action, name="follow-service-stream"):
    """`tail -F` every file the service's log stream currently has.

    Resolution happens here rather than in `plan()` because a plan describes
    and does not act -- listing a directory to build the argv would make
    `--dry-run` touch the machine.

    `-F` rather than `-f` so a file being rotated out from under the tail is
    reopened by name. A file created after this starts -- tomorrow's, at
    midnight -- is not picked up; following a stream that has not been written
    yet needs a watcher, and this is `just logs`, not the support bundle.
    """

    def render(self) -> str:
        return "tail -F the service log stream under ~/.capsem/run"

    def perform(self, context: Context) -> None:
        stream = host.home() / context.config.logs.service_log
        # `service.log` and `service.<date>.log`, never `services.log`. The
        # unrotated name still exists on installs that predate rotation, and
        # raw stderr -- panics, death before tracing starts -- lands there.
        files = sorted(
            path
            for path in stream.parent.glob(f"{stream.stem}*{stream.suffix}")
            if path.is_file()
            and (path.name == stream.name or path.name.startswith(f"{stream.stem}."))
        )
        if not files:
            raise GateError(
                f"no log stream under {stream.parent}; is the service running?"
            )
        Run(["tail", "-F", *(str(path) for path in files)]).perform(context)
