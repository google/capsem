"""Every gate starts by reaping what earlier runs leaked. Nobody does it by hand.

`OrphanAccounting` reaps at the *end* of a run and forgives whatever existed
before it, so a process that outlived a killed run was forgiven by every run
after. Eleven sccache servers accumulated that way, the oldest eight days old:
nine claimed one socket path, of which at most one was reachable, and two
served gate prefixes that had since been deleted. `--stop-server` only ever
reaches the one server still bound to the socket.

The reaper is scoped by the socket a server was started for, read from its own
environment, so another project's sccache on the same machine is not ours to
kill.
"""

from __future__ import annotations

import contextlib
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Iterator
from pathlib import Path

import psutil
import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import preflight
from capsem_builder.gate.cachetooling import CompilerCache
from capsem_builder.gate.reaper import StaleProcesses
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)
MODULE = CONFIG.toolchain.reaper_module
PROGRAM = CONFIG.toolchain.compiler_cache_command
SOCKET_ENV = CONFIG.environment.sccache_server_uds


pytestmark = pytest.mark.skipif(
    shutil.which(PROGRAM) is None, reason=f"{PROGRAM} is not installed on this machine"
)


@pytest.fixture
def short(tmp_path: Path) -> Iterator[Path]:
    """A root short enough for a unix socket path, which the kernel caps at 104 bytes."""
    root = Path(tempfile.mkdtemp(prefix="capsem-reap-", dir="/tmp")).resolve()
    try:
        yield root
    finally:
        shutil.rmtree(root, ignore_errors=True)


def _serving(socket: Path) -> list[psutil.Process]:
    found = []
    for proc in psutil.process_iter():
        with contextlib.suppress(psutil.Error, OSError, SystemError):
            if proc.name() == PROGRAM and proc.environ().get(SOCKET_ENV) == str(socket):
                found.append(proc)
    return found


def _start(socket: Path) -> None:
    """What a build does: ask for a server, and leave it running."""
    socket.parent.mkdir(parents=True, exist_ok=True)
    environment = {
        **os.environ,
        SOCKET_ENV: str(socket),
        CONFIG.environment.sccache_dir: str(socket.parent / "store"),
        CONFIG.environment.sccache_idle_timeout: "0",
    }
    started = subprocess.run(
        [PROGRAM, "--start-server"], env=environment, check=False, capture_output=True, text=True
    )
    assert started.returncode == 0, started.stderr


@contextlib.contextmanager
def _server(socket: Path) -> Iterator[psutil.Process]:
    """The real server, daemonized the way a build leaves it: not a double.

    Two doubles were tried first. macOS kills a copied platform binary on exec,
    and an interpreter started through a link renames itself -- so both passed
    or failed for reasons that had nothing to do with the reaper.
    """
    before = {proc.pid for proc in _serving(socket)}
    _start(socket)
    (server,) = [proc for proc in _serving(socket) if proc.pid not in before]
    try:
        yield server
    finally:
        with contextlib.suppress(psutil.Error):
            server.kill()


def _reap(cache_root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", MODULE, "--program", PROGRAM, "--socket-environment", SOCKET_ENV,
         "--cache-root", str(cache_root), "--grace-seconds", "2"],
        capture_output=True, text=True, check=False, timeout=60,
    )


def test_a_server_left_by_an_earlier_run_is_reaped_and_named(short: Path) -> None:
    cache = short / "cache"
    with _server(cache / "tools/sccache.sock") as stale:
        result = _reap(cache)
        assert result.returncode == 0, result.stderr
        assert not stale.is_running()
        assert str(stale.pid) in result.stdout, "a reaped process must be named in the run log"


def test_every_server_on_one_socket_is_reaped_not_just_the_reachable_one(short: Path) -> None:
    """`--stop-server` reaches one. Nine sat on one path, eight of them deaf."""
    cache = short / "cache"
    socket = cache / "tools/sccache.sock"
    with _server(socket) as deaf:
        # The path is freed and a second server binds it. The first is still
        # alive and can never be reached again: that is the leak.
        socket.unlink()
        with _server(socket) as reachable:
            assert deaf.pid != reachable.pid
            assert _reap(cache).returncode == 0
            assert not deaf.is_running(), "the unreachable server survived"
            assert not reachable.is_running()


def test_a_server_for_a_deleted_gate_prefix_is_reaped(short: Path) -> None:
    cache = short / "cache"
    prefix = cache / "worktrees/070c30ca"
    with _server(prefix / "sccache.sock") as stale:
        shutil.rmtree(prefix)
        assert _reap(cache).returncode == 0
        assert not stale.is_running()


def test_another_projects_server_is_not_ours_to_kill(short: Path) -> None:
    with _server(short / "elsewhere/sccache.sock") as theirs:
        assert _reap(short / "cache").returncode == 0
        assert theirs.is_running()


def test_a_sibling_directory_sharing_the_prefix_is_not_inside_the_cache(short: Path) -> None:
    with _server(short / "cache-of-someone-else/sccache.sock") as theirs:
        assert _reap(short / "cache").returncode == 0
        assert theirs.is_running()


def test_a_different_program_is_never_reaped_whatever_its_environment(short: Path) -> None:
    cache = short / "cache"
    with subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(300)"],
        env={**os.environ, SOCKET_ENV: str(cache / "sccache.sock")},
    ) as other:
        try:
            assert _reap(cache).returncode == 0
            assert other.poll() is None
        finally:
            other.kill()


def test_every_exclusive_command_reaps_before_it_starts_its_own_server() -> None:
    held = preflight.holdings(CONFIG, RecordingRunner(ROOT), "test-fast", exclusive=True, declared=())
    kinds = [type(resource) for resource in held]
    assert StaleProcesses in kinds, "an exclusive command started without reaping"
    assert kinds.index(StaleProcesses) < kinds.index(CompilerCache), (
        "reaping after the server starts kills the server this run depends on"
    )


def test_a_command_without_the_machine_reaps_nothing() -> None:
    """Without the lock, a live server may belong to a run that is using it."""
    held = preflight.holdings(CONFIG, RecordingRunner(ROOT), "lint", exclusive=False, declared=())
    assert StaleProcesses not in [type(resource) for resource in held]


def test_asking_what_a_plan_would_do_reaps_nothing() -> None:
    runner = RecordingRunner(ROOT)
    assert runner.observing
    StaleProcesses(CONFIG, runner).acquire()
    assert not runner.commands
