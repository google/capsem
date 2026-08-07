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

from pathlib import Path

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
    report_stop_timeout = 5.0


class _Config:
    def __init__(self, root: Path) -> None:
        self.sandbox = _Settings()
        self.runlog = type("RunLog", (), {"root": "runs"})()
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
    or writes nowhere, and a fake streamer proves neither. This asserts the
    capture file exists and the summary is derived from it -- the streamer
    may legitimately observe no sandbox traffic during the test.
    """
    runner = RecordingRunner(PROJECT_ROOT)
    config = _Config(tmp_path)
    resource = sandboxreport.SandboxReport(config, runner, mode=sandbox.REPORT)

    resource.acquire()
    assert resource._stream is not None, "report mode started no streamer"
    assert resource._stream.poll() is None, "the streamer exited immediately"
    resource.release()

    capture = tmp_path / "runs" / _Settings.report_log_name
    assert capture.is_file(), "report mode left no capture"
    summary = capture.with_suffix(_Settings.report_summary_suffix)
    assert summary.is_file(), "release wrote no allow-list"
    assert resource._stream is None, "the streamer outlived the run"


def test_the_collector_is_part_of_the_complete_gate() -> None:
    """A resource nothing constructs measures nothing, which is the shape the
    original defect had: mechanism present, never reached."""
    source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "gateresources.py").read_text()
    assert "SandboxReport(config, runner, mode=mode)" in source
