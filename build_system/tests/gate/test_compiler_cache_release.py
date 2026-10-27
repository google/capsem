"""Concurrent final users must not stop the same real cache server twice."""

from __future__ import annotations

import contextlib
import socket as unix_socket
import threading
from collections.abc import Iterable, Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import psutil
import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.cache.leases import exclusive_paths
from capsem_builder.gate import cachetooling
from capsem_builder.gate.invocation import Command
from capsem_builder.gate.proc import Completed, Runner
from test_stale_process_reaper import CONFIG, ROOT, SOCKET_ENV, _serving
from test_stale_process_reaper import short as short


def test_last_release_waits_for_a_real_connected_client_to_close(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A real accepted connection keeps sccache alive after its shutdown reply."""
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    acknowledged = threading.Event()

    class ShutdownRunner(Runner):
        def execute(self, command: Command) -> Completed:
            result = super().execute(command)
            if command.argv[-1] == "--stop-server":
                acknowledged.set()
            return result

    resource = cachetooling.CompilerCache(CONFIG, ShutdownRunner(ROOT))
    socket = Path(resource.environment()[SOCKET_ENV])
    resource.acquire()
    (server,) = _serving(socket)
    try:
        with unix_socket.socket(unix_socket.AF_UNIX, unix_socket.SOCK_STREAM) as client, ThreadPoolExecutor(max_workers=1) as pool:
            client.connect(str(socket))
            released = pool.submit(resource.release)
            try:
                assert acknowledged.wait(timeout=10), "the real shutdown request never completed"
                assert server.is_running(), "the client must keep the real daemon in its shutdown phase"
                with pytest.raises(TimeoutError):
                    released.result(timeout=0.1)
            finally:
                client.close()
                released.result(timeout=15)
        assert not server.is_running(), "release returned before its daemon exited"
    finally:
        resource.release()
        with contextlib.suppress(psutil.Error):
            server.kill()


def test_concurrent_last_releases_stop_the_server_once(
    short: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(load_policy(ROOT).authority_environment, str(short))
    first = cachetooling.CompilerCache(CONFIG, Runner(ROOT))
    second = cachetooling.CompilerCache(CONFIG, Runner(ROOT))
    socket = Path(first.environment()[SOCKET_ENV])
    first.acquire()
    second.acquire()
    first_at_mutation, second_at_mutation, first_done = (threading.Event() for _ in range(3))

    @contextlib.contextmanager
    def ordered_mutation(paths: Iterable[Path]) -> Iterator[bool]:
        # Preserve real flocks and the real server. Delay the second mutation
        # attempt until the first release finishes. Before release serialized
        # its lease drop, both users could leave before either checked for idle.
        if threading.current_thread().name == "cache-release-first":
            first_at_mutation.set()
            second_at_mutation.wait(timeout=1)
        else:
            second_at_mutation.set()
            assert first_done.wait(timeout=10)
        with exclusive_paths(paths) as idle:
            yield idle

    def release_first() -> None:
        threading.current_thread().name = "cache-release-first"
        try:
            first.release()
        finally:
            first_done.set()

    monkeypatch.setattr(cachetooling, "exclusive_paths", ordered_mutation)
    try:
        with ThreadPoolExecutor(max_workers=2) as pool:
            early = pool.submit(release_first)
            assert first_at_mutation.wait(timeout=10)
            late = pool.submit(second.release)
            early.result(timeout=15)
            late.result(timeout=15)
        assert not _serving(socket), "the last user left a cache server behind"
    finally:
        monkeypatch.setattr(cachetooling, "exclusive_paths", exclusive_paths)
        first.release()
        second.release()
