"""What the sandbox would have refused, collected while the gate runs.

The measuring half of report mode, and the half that was missing: the profile
was generated and applied, but nothing captured its output, so a full
report-mode gate run produced an `errors.log` with **zero** sandbox entries.
Report mode without a collector measures nothing -- and the enforcing mode it
exists to prepare cannot be reached without the list it was supposed to yield.

`(allow ... (with report))` permits the operation and writes it to the unified
log under the `Sandbox` sender:

    Sandbox: curl(56701) allow network-outbound remote:*:443
    Sandbox: curl(56701) allow network-outbound /private/var/run/mDNSResponder

Process, pid, operation, resource. Deduplicated across a run, that *is* the
allow-list an enforcing profile needs, which is why one report run replaces
the forty enforcing runs that would each discover one more thing at a
different minute.

Bounded by two things and not by a size cap. The predicate is a single sender,
so the stream is sandbox decisions rather than the system log; and the
streamer is terminated on release down every path, including failure -- a
collector that outlives its run is exactly the leak orphan accounting exists
to catch. The capture lands in the run directory, so `runhistory` rotation
reclaims it with everything else that run produced.
"""

from __future__ import annotations

import re
import subprocess
from collections import Counter
from pathlib import Path

from .lifecycle import Resource
from .proc import Runner
from .sandbox import REPORT

#: `Sandbox: <process>(<pid>) <decision> <operation> <resource>`. The resource
#: is optional -- a bare `allow network-outbound` carries no path -- and the
#: process name is kept because "which command reached for this" is most of
#: what makes the list actionable.
_ENTRY = re.compile(
    r"Sandbox:\s+(?P<process>\S+?)\((?P<pid>\d+)\)\s+"
    r"(?P<decision>allow|deny)\s+(?P<operation>[\w\-*]+)"
    r"(?:\s+(?P<resource>.+))?$"
)


class SandboxReport(Resource, name="sandbox-report"):
    """Stream `Sandbox` log entries for the life of a report-mode run.

    Does nothing in the other two modes. In `off` there is no profile, and in
    `enforce` the profile is the answer rather than the question -- a denial
    there stops the run, which is louder than any log line.
    """

    def __init__(self, config, runner: Runner, *, mode: str) -> None:
        self._settings = config.sandbox
        self._runner = runner
        self._mode = mode
        self._target = config.path(config.runlog.root) / self._settings.report_log_name
        self._stream: subprocess.Popen | None = None
        self._handle = None

    def acquire(self) -> None:
        if self._mode != REPORT:
            return
        self._target.parent.mkdir(parents=True, exist_ok=True)
        self._handle = self._target.open("w", encoding="utf-8")
        # Not through `Runner`: this outlives the call that starts it, and the
        # runner's exec accounting is for commands that finish.
        self._stream = subprocess.Popen(
            [
                self._settings.log_command,
                "stream",
                "--style",
                self._settings.report_style,
                "--predicate",
                self._settings.report_predicate,
            ],
            stdout=self._handle,
            stderr=subprocess.STDOUT,
        )
        self._runner.step(f"Collecting sandbox report entries into {self._target}")

    def release(self) -> None:
        stream, self._stream = self._stream, None
        if stream is not None:
            stream.terminate()
            try:
                stream.wait(timeout=self._settings.report_stop_timeout)
            except subprocess.TimeoutExpired:
                # A streamer that ignores SIGTERM must still not outlive the
                # run: this is the leak the orphan accounting would report,
                # and blaming a later run for it costs an hour to diagnose.
                stream.kill()
                stream.wait()
        if self._handle is not None:
            self._handle.close()
            self._handle = None
        if self._mode == REPORT:
            self._summarize()

    def _summarize(self) -> None:
        """Write the deduplicated allow-list beside the raw capture.

        The raw stream is evidence; this is the thing a person acts on. Sorted
        by frequency because the operations a gate reaches for constantly are
        the ones an enforcing profile must name first.
        """
        if not self._target.is_file():
            return
        seen = observed(self._target.read_text(encoding="utf-8"))
        summary = self._target.with_suffix(self._settings.report_summary_suffix)
        lines = [f"{count:>6}  {operation}  {resource}" for (operation, resource), count in seen]
        summary.write_text(
            "\n".join(lines) + ("\n" if lines else ""),
            encoding="utf-8",
        )
        self._runner.step(f"{len(lines)} distinct sandbox operation(s) recorded in {summary}")


def observed(captured: str) -> list[tuple[tuple[str, str], int]]:
    """Distinct `(operation, resource)` pairs, most frequent first.

    A free function so the parsing can be tested against real captured text
    without starting a streamer -- the part that decides what the allow-list
    says is the part worth pinning.
    """
    counts: Counter[tuple[str, str]] = Counter()
    for line in captured.splitlines():
        match = _ENTRY.search(line)
        if match is None:
            continue
        counts[(match["operation"], (match["resource"] or "").strip())] += 1
    return counts.most_common()


def capture_path(config) -> Path:
    """Where a report-mode run leaves its capture."""
    return config.path(config.runlog.root) / config.sandbox.report_log_name
