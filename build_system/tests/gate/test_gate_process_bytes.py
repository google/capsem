"""Source protocol children, never product workloads or benchmarks."""

import os
import sys
from pathlib import Path

import psutil
import pytest
from capsem_builder.gate import processgroup
from capsem_builder.gate.errors import GateError

POLICY = processgroup.StopPolicy(grace_seconds=1, poll_seconds=0.01)


def run(tmp_path, **kwargs):
    return processgroup.tee(
        [sys.executable, "-c", "import os; os.write(1,b'\\xffexact\\x00')"],
        cwd=tmp_path,
        env=os.environ,
        policy=POLICY,
        timeout_seconds=2,
        **kwargs,
    )


def test_byte_callback_preserves_original_non_utf8_and_nul(tmp_path):
    chunks = []
    assert run(tmp_path, write_bytes=chunks.append) == 0
    assert b"".join(chunks) == b"\xffexact\x00"


def test_existing_text_callback_keeps_its_behavior(tmp_path):
    chunks = []
    assert run(tmp_path, write=chunks.append) == 0
    assert "".join(chunks) == "\ufffdexact\x00"


@pytest.mark.parametrize("callbacks", [{}, {"write": str, "write_bytes": bytes}])
def test_callback_selection_fails_before_child(tmp_path, monkeypatch, callbacks):
    monkeypatch.setattr(processgroup.subprocess, "Popen", lambda *a, **k: pytest.fail("child"))
    with pytest.raises(ValueError, match="exactly one"):
        run(tmp_path, **callbacks)


@pytest.mark.parametrize("failure", ["callback", "deadline"])
def test_owned_child_is_retired_on_callback_failure_or_deadline(tmp_path, failure):
    pidfile = tmp_path / "pid"
    script = (
        f"import os,time; open({str(pidfile)!r},'w').write(str(os.getpid())); "
        "os.write(1,b'ready'); time.sleep(60)"
    )

    def write(chunk):
        if failure == "callback":
            raise ValueError("budget exceeded")

    expected = ValueError if failure == "callback" else GateError
    with pytest.raises(expected):
        processgroup.tee(
            [sys.executable, "-c", script],
            cwd=tmp_path,
            env=os.environ,
            policy=POLICY,
            timeout_seconds=0.5,
            write_bytes=write,
        )
    assert not psutil.pid_exists(int(pidfile.read_text()))


def test_protocol_stdout_can_be_kept_separate_from_diagnostics(tmp_path, capfd):
    chunks = []
    assert (
        processgroup.tee(
            [sys.executable, "-c", "import os; os.write(1,b'{}'); os.write(2,b'diagnostic')"],
            cwd=tmp_path,
            env=os.environ,
            policy=POLICY,
            write_bytes=chunks.append,
            stderr=None,
            timeout_seconds=2,
        )
        == 0
    )
    assert b"".join(chunks) == b"{}" and "diagnostic" in capfd.readouterr().err


def test_explicit_executable_preserves_original_argv0(tmp_path):
    chunks = []
    original = "/original/cargo/argv0"
    assert (
        processgroup.tee(
            [original, "-c", "import sys; print(sys.orig_argv[0])"],
            executable=Path(sys.executable),
            cwd=tmp_path,
            env=os.environ,
            policy=POLICY,
            write_bytes=chunks.append,
            timeout_seconds=2,
        )
        == 0
    )
    assert b"".join(chunks).decode().strip() == original


def test_undrained_diagnostic_pipe_fails_before_child(tmp_path, monkeypatch):
    monkeypatch.setattr(processgroup.subprocess, "Popen", lambda *a, **k: pytest.fail("child"))
    with pytest.raises(ValueError, match="undrained"):
        run(tmp_path, write_bytes=bytes, stderr=-1)
