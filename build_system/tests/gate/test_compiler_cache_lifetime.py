"""A real paused compiler client must survive unrelated gate cache cleanup."""

from __future__ import annotations

import contextlib
import shutil
import subprocess
import sys
import threading
import time
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import psutil
import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import exclusive_paths
from capsem_builder.gate import cachelayout
from capsem_builder.gate.cachetooling import CompilerCache
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.proc import Runner
from helpers.bounded import WRAPPER, bounded
from test_stale_process_reaper import CONFIG, PROGRAM, ROOT, SOCKET_ENV, _reap, _server, _serving
from test_stale_process_reaper import short as short


@contextlib.contextmanager
def _compiling(root: Path, *, opaque: bool = False) -> Iterator[subprocess.Popen[str]]:
    """Pause at the real rustc invocation, then compile and execute real bytes.

    The proxy forwards compiler discovery unchanged. Only this fixture's source
    waits at a file barrier; neither sccache nor its process ownership is faked.
    """
    rustc = shutil.which("rustc")
    assert rustc is not None
    source = root / "probe.rs"
    source.write_text('fn main() { println!("compiler-survived"); }\n')
    ready, proceed = root / "ready", root / "proceed"
    output = root / "probe"
    proxy = root / "rustc-proxy"
    proxy.write_text(
        f"#!{sys.executable}\n"
        "import os, sys, time\nfrom pathlib import Path\n"
        f"if {str(source)!r} in sys.argv:\n"
        f"    Path({str(ready)!r}).write_text(str(os.getpid()))\n"
        f"    while not Path({str(proceed)!r}).exists(): time.sleep(0.01)\n"
        f"os.execv({rustc!r}, [{rustc!r}, *sys.argv[1:]])\n"
    )
    proxy.chmod(0o700)
    command = [PROGRAM, str(proxy), "--crate-name", "probe", str(source), "-o", str(output)]
    argv = bounded(command, 30)
    if opaque:
        code = f"import subprocess; subprocess.run({command!r}, check=True)"
        argv = [sys.executable, str(WRAPPER), "--timeout-seconds", "30", "--compiler-cache", "--", sys.executable, "-c", code]
    with subprocess.Popen(
        argv,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    ) as client:
        try:
            deadline = time.monotonic() + 15
            while not ready.exists() and client.poll() is None and time.monotonic() < deadline:
                time.sleep(0.01)
            assert ready.exists(), "the real compiler request never reached its barrier"
            yield client
        finally:
            failure = sys.exc_info()[0]
            proceed.touch()
            stdout, stderr = client.communicate(timeout=40)
            if failure is None:
                assert client.returncode == 0, stdout + stderr
                assert stdout == "", "cache administration polluted the compiler's stdout"
                ran = subprocess.run([str(output)], check=True, capture_output=True, text=True)
                assert ran.stdout == "compiler-survived\n"


@pytest.mark.parametrize("transition", ["reap", "acquire", "release"])
@pytest.mark.parametrize("opaque", [False, True], ids=["direct", "opaque-driver"])
def test_unleased_compiler_survives_gate_cache_lifecycle(
    short: Path, monkeypatch: pytest.MonkeyPatch, transition: str, opaque: bool,
) -> None:
    policy = load_policy(ROOT)
    monkeypatch.setenv(policy.authority_environment, str(short))
    socket = Path(CompilerCache(CONFIG, Runner(ROOT)).environment()[SOCKET_ENV])
    resource = CompilerCache(CONFIG, Runner(ROOT))
    with _server(socket) as server:
        try:
            if transition == "release":
                resource.acquire()
                assert [proc.pid for proc in _serving(socket)] == [server.pid]
            with _compiling(short, opaque=opaque):
                endpoint = socket.stat()
                if transition == "reap":
                    result = _reap(cachelayout.cache_paths(CONFIG).root)
                    assert result.returncode == 0, result.stderr
                elif transition == "acquire":
                    resource.acquire()
                else:
                    resource.release()
                assert server.is_running(), f"gate {transition} killed a live compiler's server"
                current = socket.stat()
                assert (current.st_dev, current.st_ino) == (endpoint.st_dev, endpoint.st_ino), (
                    "a second daemon replaced the live socket"
                )
        finally:
            resource.release()


def test_a_killed_bounded_owner_does_not_protect_its_server_forever(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    policy = load_policy(ROOT)
    monkeypatch.setenv(policy.authority_environment, str(short))
    socket = Path(CompilerCache(CONFIG, Runner(ROOT)).environment()[SOCKET_ENV])
    ready = short / "owner-ready"
    code = f"import os,time; from pathlib import Path; Path({str(ready)!r}).write_text(str(os.getpid())); time.sleep(60)"
    command = [sys.executable, str(WRAPPER), "--timeout-seconds", "30", "--compiler-cache", "--", sys.executable, "-c", code]
    with _server(socket) as server, subprocess.Popen(command) as owner:
        child = None
        try:
            deadline = time.monotonic() + 10
            while not ready.exists() and owner.poll() is None and time.monotonic() < deadline:
                time.sleep(0.01)
            assert ready.exists()
            child = psutil.Process(int(ready.read_text()))
            owner.kill()
            owner.wait(timeout=5)
            child.kill()
            psutil.wait_procs([child], timeout=5)
            result = _reap(cachelayout.cache_paths(CONFIG).root)
            assert result.returncode == 0, result.stderr
            assert not server.is_running(), "a dead bounded owner retained a live-use lease"
        finally:
            if owner.poll() is None:
                owner.kill()
                owner.wait(timeout=5)
            if child is not None:
                with contextlib.suppress(psutil.Error):
                    child.kill()


def test_cache_acquisition_starts_one_server_and_last_release_stops_it(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    first, second = CompilerCache(CONFIG, Runner(ROOT)), CompilerCache(CONFIG, Runner(ROOT))
    socket = Path(first.environment()[SOCKET_ENV])
    try:
        first.acquire()
        (server,) = _serving(socket)
        second.acquire()
        assert [proc.pid for proc in _serving(socket)] == [server.pid]
        first.release()
        assert server.is_running(), "another resource still owns the cache"
        second.release()
        assert not server.is_running(), "the last gate user left a daemon behind"
    finally:
        first.release()
        second.release()
        for proc in _serving(socket):
            with contextlib.suppress(psutil.Error):
                proc.kill()


def test_simultaneous_cold_users_share_one_server(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    resources = [CompilerCache(CONFIG, Runner(ROOT)) for _ in range(4)]
    socket = Path(resources[0].environment()[SOCKET_ENV])
    barrier = threading.Barrier(len(resources), timeout=10)

    def acquire(resource: CompilerCache) -> None:
        barrier.wait()
        resource.acquire()

    try:
        with ThreadPoolExecutor(max_workers=len(resources)) as pool:
            list(pool.map(acquire, resources))
        assert len(_serving(socket)) == 1, "concurrent bootstrap stranded extra daemons"
    finally:
        for resource in resources:
            resource.release()
        for proc in _serving(socket):
            with contextlib.suppress(psutil.Error):
                proc.kill()


def test_failed_acquisition_releases_its_cache_use(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    runner = Runner(ROOT)

    def fail(*args: object, **kwargs: object) -> str:
        raise GateError("fixture startup failure")

    monkeypatch.setattr(runner, "capture", fail)
    resource = CompilerCache(CONFIG, runner)
    with pytest.raises(GateError, match="fixture startup failure"):
        resource.acquire()
    paths = cachelayout.cache_paths(CONFIG)
    with exclusive_paths(paths.stage("rust-sccache") / lock for lock in paths.policy.stages["rust-sccache"].mutation_locks) as idle:
        assert idle, "failed startup left a shared cache user behind"


def test_bounded_inspection_does_not_lock_its_own_cache_cleanup(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    code = (
        "from pathlib import Path; from capsem_builder.cache.config import load_paths; "
        "from capsem_builder.cache.leases import exclusive_paths; "
        f"p=load_paths(Path({str(ROOT)!r})); "
        "locks=[p.stage('rust-sccache')/lock for lock in p.policy.stages['rust-sccache'].mutation_locks]; "
        "\nwith exclusive_paths(locks) as idle: assert idle, 'inspection blocked its own cleanup'\n"
    )
    result = subprocess.run(bounded([sys.executable, "-c", code], 30), capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stdout + result.stderr
    socket = Path(CompilerCache(CONFIG, Runner(ROOT)).environment()[SOCKET_ENV])
    assert not _serving(socket), "ordinary inspection unexpectedly started a cache daemon"


def test_keyed_clippy_retains_the_original_compiler_lifetime(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem_builder.gate.tools.ci import run_bounded_command

    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    monkeypatch.setattr(run_bounded_command, "_repository_root", lambda: ROOT)
    keyed = [sys.executable, "-c", "print('keyed clippy')"]
    monkeypatch.setattr(run_bounded_command, "_keyed_clippy", lambda *_: (keyed, {}))

    def run(command: list[str], *_: object) -> int:
        assert command == keyed
        paths = cachelayout.cache_paths(CONFIG)
        with exclusive_paths(paths.stage("rust-sccache") / lock for lock in paths.policy.stages["rust-sccache"].mutation_locks) as idle:
            assert not idle, "keying Clippy hid the original Cargo compiler from cache lifetime"
        return 0

    monkeypatch.setattr(run_bounded_command, "_run", run)
    assert run_bounded_command.run(["--timeout-seconds", "30", "--", "cargo", "clippy"]) == 0
