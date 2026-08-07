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

**It cannot run inside the sandbox it measures.** `/usr/bin/log` refuses
outright -- `log: Cannot run while sandboxed` -- so a collector started from a
`Resource`, which is already inside the re-exec'd sandboxed process, captures
32 bytes of that refusal and nothing else. It is therefore started in
`sandbox.applied`, immediately *before* the process replaces itself with the
sandboxed one: a child that already exists is not retroactively sandboxed, so
it keeps streaming for the whole run. Its pid goes in a file, and the resource
inside the sandbox reads that file to stop it and summarize.

Bounded by two things and not by a size cap. The predicate is a single sender,
so the stream is sandbox decisions rather than the system log; and the
streamer is terminated on release down every path, including failure -- a
collector that outlives its run is exactly the leak orphan accounting exists
to catch. The capture lands in the run directory, so `runhistory` rotation
reclaims it with everything else that run produced.
"""

from __future__ import annotations

import contextlib
import os
import re
import signal
import subprocess
import time
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
        # Into *this run's* directory, not the history root. `_recording()`
        # allocates it and repoints `latest` before any resource is acquired,
        # so by the time this runs the link is live. The root would have meant
        # each run overwriting the last capture, and -- worse for a file that
        # grows -- one that `runhistory` rotation never reclaims, because
        # rotation removes run directories and this would not be in one.
        history = config.path(config.runlog.root)
        current = history / config.runlog.latest_link
        self._target = (current if current.is_dir() else history) / self._settings.report_log_name

    def acquire(self) -> None:
        """Nothing to start: it is already running, and had to be.

        See the module docstring -- `log` refuses to run sandboxed, and this
        object only exists inside the sandbox.
        """

    def release(self) -> None:
        if self._mode != REPORT:
            return
        pidfile = self._target.with_suffix(self._settings.report_pid_suffix)
        if pidfile.is_file():
            try:
                pid = int(pidfile.read_text(encoding="utf-8").strip())
            except ValueError:
                pid = 0
            if pid > 0:
                # A collector that outlives its run is the leak orphan
                # accounting exists to catch, so SIGKILL follows SIGTERM
                # rather than trusting it.
                for signal_number in (signal.SIGTERM, signal.SIGKILL):
                    try:
                        os.kill(pid, signal_number)
                    except ProcessLookupError:
                        break
                    time.sleep(self._settings.report_stop_timeout / 10)
                    try:
                        os.kill(pid, 0)
                    except ProcessLookupError:
                        break
            pidfile.unlink(missing_ok=True)
            _reap(pid)
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


def _reap_current() -> None:
    """Stop and wait for whatever streamer this process still holds."""
    global _STREAM
    stream, _STREAM = _STREAM, None
    if stream is None:
        return
    with contextlib.suppress(OSError):
        stream.terminate()
    with contextlib.suppress(OSError, subprocess.TimeoutExpired):
        stream.wait(timeout=5)


def _reap(pid: int) -> None:
    """Wait for the streamer if it was ours, so it does not linger as a zombie.

    In the gate it is not ours -- the process that started it was replaced by
    `exec`, so it reparents to init and init reaps it. In a test, and in any
    run where the two happen in one process, the killed child stays a zombie
    until someone waits, and Python complains from `Popen.__del__` about a
    subprocess still running.
    """
    global _STREAM
    stream, _STREAM = _STREAM, None
    if stream is not None and stream.pid == pid:
        with contextlib.suppress(OSError):
            stream.wait(timeout=5)


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
    """Where a report-mode run leaves its capture, resolved to the live run."""
    history = config.path(config.runlog.root)
    current = history / config.runlog.latest_link
    return (current if current.is_dir() else history) / config.sandbox.report_log_name


#: The streamer, kept referenced for as long as this process lives.
#:
#: Deliberately outliving the call that starts it is the whole design, and
#: Python reads an abandoned `Popen` as a mistake: dropping the handle emits
#: `ResourceWarning: subprocess N is still running` from the deallocator. In
#: the gate that warning goes nowhere, and under pytest it becomes an error --
#: so the intent is spelled with a reference rather than argued about.
_STREAM: subprocess.Popen | None = None


def start_outside_the_sandbox(config, runner) -> None:
    """Begin streaming before this process becomes a sandboxed one.

    Called from `sandbox.applied`, which is the last moment anything here runs
    unsandboxed. The child survives the `exec` that follows and is not covered
    by the profile the replacement adopts, which is the only arrangement in
    which `log` runs at all.
    """
    # Any predecessor first. One process should only ever start one of these,
    # but if it starts a second the first is orphaned -- a running child with
    # nobody holding it, which Python reports from `Popen.__del__` as a
    # subprocess still running and which is a genuine leak, not a warning.
    _reap_current()

    settings = config.sandbox
    target = capture_path(config)
    target.parent.mkdir(parents=True, exist_ok=True)
    handle = target.open("w", encoding="utf-8")
    global _STREAM
    stream = subprocess.Popen(
        [
            settings.log_command,
            "stream",
            "--style",
            settings.report_style,
            "--predicate",
            settings.report_predicate,
        ],
        stdout=handle,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    # The child has its own descriptor now, so this one is ours to close. An
    # abandoned handle is the same mistake as an abandoned `Popen`: harmless
    # in the gate, an error under pytest, and in both cases a claim that
    # something is still in use when it is not.
    handle.close()
    _STREAM = stream
    target.with_suffix(settings.report_pid_suffix).write_text(str(stream.pid), encoding="utf-8")
    runner.step(f"Collecting sandbox report entries into {target}")
