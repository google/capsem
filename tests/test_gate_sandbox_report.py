"""Report mode has to actually record something.

The defect this pins is not a wrong answer but a missing one. The profile was
generated and applied for a whole 60-minute report-mode gate run, and the
resulting `errors.log` held **zero** sandbox entries, because nothing was
reading the unified log the `(with report)` modifier writes to. Report mode
that collects nothing is indistinguishable from a clean run, and the enforcing
profile it exists to prepare can never be built from it.

The parser is tested against real captured output rather than invented lines.
The three below came from `/usr/bin/log stream --predicate 'sender ==
"Sandbox"'` while a `curl` ran under a report profile.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import sandbox, sandboxreport

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: Real output, not a fixture shaped to suit the regex.
CAPTURED = """
Sandbox: curl(56701) allow network-outbound /private/var/run/mDNSResponder
Sandbox: curl(56701) allow network-outbound
Sandbox: curl(56701) allow network-outbound remote:*:443
Sandbox: capsem-gate(1234) deny file-write-data /Users/elie/git/capsem/config
Sandbox: curl(56999) allow network-outbound remote:*:443
""".strip()


def test_the_allow_list_is_the_distinct_operations_not_the_line_count() -> None:
    """Deduplicated across processes: two curls reaching the same host is one
    rule an enforcing profile has to carry, not two."""
    seen = dict(sandboxreport.observed(CAPTURED))

    assert seen[("network-outbound", "remote:*:443")] == 2
    assert seen[("network-outbound", "/private/var/run/mDNSResponder")] == 1
    assert seen[("file-write-data", "/Users/elie/git/capsem/config")] == 1


def test_an_operation_with_no_resource_is_still_recorded() -> None:
    """`allow network-outbound` with nothing after it is a real line, and
    dropping it would silently shorten the allow-list."""
    assert ("network-outbound", "") in dict(sandboxreport.observed(CAPTURED))


def test_ordinary_log_noise_is_not_mistaken_for_a_decision() -> None:
    noise = "Sandbox: something entirely unlike a decision line\nunrelated\n"
    assert sandboxreport.observed(noise) == []


def test_most_frequent_first() -> None:
    """The operations a gate reaches for constantly are the ones an enforcing
    profile must name first, so the summary is ordered by frequency."""
    assert sandboxreport.observed(CAPTURED)[0][1] == 2


class _Settings:
    log_command = "/usr/bin/log"
    report_predicate = 'sender == "Sandbox"'
    report_style = "ndjson"
    report_log_name = "sandbox-report.ndjson"
    report_summary_suffix = ".allowlist.txt"
    report_pid_suffix = ".pid"
    report_stop_timeout = 5.0


class _Config:
    def __init__(self, root: Path) -> None:
        self.sandbox = _Settings()
        self.runlog = type("RunLog", (), {"root": "runs", "latest_link": "latest"})()
        self._root = root

    def path(self, relative: str) -> Path:
        return self._root / relative


def test_off_and_enforce_start_no_collector(tmp_path: Path) -> None:
    """Only report mode measures. In `off` there is no profile at all, and in
    `enforce` a denial stops the run, which is louder than any log line."""
    for mode in (sandbox.OFF, sandbox.ENFORCE):
        runner = RecordingRunner(PROJECT_ROOT)
        resource = sandboxreport.SandboxReport(_Config(tmp_path), runner, mode=mode)
        resource.acquire()
        resource.release()
        assert runner.notes == [], f"{mode} started a collector"
        assert not (tmp_path / "runs" / _Settings.report_log_name).exists()


def test_report_mode_captures_and_summarizes(tmp_path: Path) -> None:
    """The real thing, against the real `log` binary.

    Not mocked: the failure being prevented is that the command does not run
    or writes nowhere, and a fake streamer proves neither.

    Started through `start_outside_the_sandbox` rather than through the
    resource, because that is where it has to happen -- `log` refuses to run
    inside a sandbox, and the resource only exists on the far side of the
    re-exec that applies one.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    config = _Config(tmp_path)

    sandboxreport.start_outside_the_sandbox(config, runner)
    capture = tmp_path / "runs" / _Settings.report_log_name
    pidfile = capture.with_suffix(_Settings.report_pid_suffix)
    assert pidfile.is_file(), "no streamer pid was recorded for the sandbox to stop"
    pid = int(pidfile.read_text(encoding="utf-8").strip())

    sandboxreport.SandboxReport(config, runner, mode=sandbox.REPORT).release()

    assert capture.is_file(), "report mode left no capture"
    assert capture.with_suffix(_Settings.report_summary_suffix).is_file(), (
        "release wrote no allow-list"
    )
    assert not pidfile.exists(), "the pidfile outlived the run"
    # Release reaps the child it killed, so the pid is genuinely gone rather
    # than a zombie that `kill(pid, 0)` would still report as present. In the
    # gate the collector reparents to init and init reaps it; in one process
    # the resource has to do it, or Python complains from `Popen.__del__`.
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)


def test_the_collector_is_part_of_the_complete_gate() -> None:
    """A resource nothing constructs measures nothing, which is the shape the
    original defect had: mechanism present, never reached."""
    source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "gateresources.py").read_text()
    assert "SandboxReport(config, runner, mode=mode)" in source


def test_the_capture_lands_in_this_runs_directory(tmp_path: Path) -> None:
    """Not in the history root.

    The root would mean each run overwriting the last capture, and a file that
    grows for a whole gate sitting where `runhistory` rotation -- which removes
    run *directories* -- would never reclaim it.
    """
    current = tmp_path / "runs" / "20260807-000000-abc123-candidate"
    current.mkdir(parents=True)
    (tmp_path / "runs" / "latest").symlink_to(current.name)

    config = _Config(tmp_path)
    runner = RecordingRunner(PROJECT_ROOT)
    sandboxreport.start_outside_the_sandbox(config, runner)
    sandboxreport.SandboxReport(config, runner, mode=sandbox.REPORT).release()

    assert (current / _Settings.report_log_name).is_file(), "capture missed the run directory"
    assert not (tmp_path / "runs" / _Settings.report_log_name).exists(), "capture went to the root"
