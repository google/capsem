import os
import pathlib
import signal
import subprocess

import pytest

TESTS_OUTPUT_DIR = pathlib.Path("/root/tests")


def pytest_ignore_collect(collection_path, config):
    """Cleanly ignore this directory if not running inside the capsem VM."""
    if os.geteuid() != 0 or not os.access("/root", os.W_OK):
        return True
    return False


@pytest.fixture(autouse=True)
def ensure_output_dir():
    """Create output directory for test artifacts."""
    TESTS_OUTPUT_DIR.mkdir(parents=True, exist_ok=True)


@pytest.fixture
def output_dir():
    """Return the shared output directory path."""
    return TESTS_OUTPUT_DIR


def _timeout_diagnostics():
    """Capture bounded guest load/process evidence while a command is stuck."""
    diagnostic_cmd = (
        "echo '--- /proc/loadavg ---'; cat /proc/loadavg 2>&1; "
        "echo '--- memory ---'; free -m 2>&1; "
        "echo '--- busiest processes ---'; "
        "ps -eo pid,ppid,stat,etime,pcpu,pmem,comm,args --sort=-pcpu 2>&1 | head -n 30"
    )
    try:
        result = subprocess.run(
            diagnostic_cmd,
            shell=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
        return result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        return "guest timeout diagnostics also exceeded 5s"


def run(cmd, timeout=10):
    """Run a shell command with bounded diagnostics and process-group cleanup."""
    process = subprocess.Popen(
        cmd,
        shell=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        diagnostics = _timeout_diagnostics()
        try:
            os.killpg(process.pid, signal.SIGTERM)
            stdout, stderr = process.communicate(timeout=2)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate()
        pytest.fail(
            f"command timed out after {timeout}s: {cmd}\n"
            f"STDOUT:\n{stdout}\nSTDERR:\n{stderr}\n{diagnostics}",
            pytrace=False,
        )
    return subprocess.CompletedProcess(cmd, process.returncode, stdout, stderr)
