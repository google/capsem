"""Reap the compiler-cache servers earlier runs leaked, before a gate starts.

`check_orphan_processes` reaps at the end of a run and forgives whatever
existed before it, so a server that outlived a killed run was forgiven by every
run after. Servers are found by name and scoped by the socket they were started
for, read from their own environment: that ties each one to a cache root, and a
server started for another project's cache is not ours to kill.

The machine lock excludes other gates, not routine focused compilers. Hold
the cache's user locks exclusively through discovery and reaping; a live
compiler's shared use preserves its server irrespective of process age.
"""

from __future__ import annotations

import argparse
import contextlib
import sys
import time
from pathlib import Path

import psutil
from capsem_builder.cache.leases import exclusive_paths
from capsem_builder.gate.processstop import is_live


def _socket_of(proc: psutil.Process, program: str, variable: str) -> Path | None:
    """The socket this server was started for, or None if it is not one.

    Reads are per-process and individually guarded: on macOS one denied
    sysctl for an unrelated system process escapes `process_iter(attrs=...)`.
    A process that cannot be read is skipped, never assumed to be ours.
    """
    try:
        if proc.name() != program:
            return None
        declared = proc.environ().get(variable)
    except (psutil.Error, OSError, SystemError):
        return None
    return Path(declared) if declared else None


def stale_servers(program: str, variable: str, cache_root: Path) -> list[tuple[psutil.Process, Path]]:
    root = cache_root.resolve()
    found = []
    for proc in psutil.process_iter():
        socket = _socket_of(proc, program, variable)
        if socket is not None and socket.resolve().is_relative_to(root):
            found.append((proc, socket))
    return found


def reap(servers: list[tuple[psutil.Process, Path]], grace_seconds: float) -> list[psutil.Process]:
    """Ask, wait, then insist. Returns whatever is somehow still alive."""
    now = time.time()
    for proc, socket in servers:
        with contextlib.suppress(psutil.Error):
            print(f"reaping stale {proc.name()} pid {proc.pid}, up {now - proc.create_time():.0f}s, for {socket}")
            proc.terminate()
    _, alive = psutil.wait_procs([proc for proc, _ in servers], timeout=grace_seconds)
    alive = [proc for proc in alive if is_live(proc)]
    for proc in alive:
        with contextlib.suppress(psutil.Error):
            proc.kill()
    _, alive = psutil.wait_procs(alive, timeout=grace_seconds)
    return [proc for proc in alive if is_live(proc)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--program", required=True)
    parser.add_argument("--socket-environment", required=True)
    parser.add_argument("--cache-root", required=True, type=Path)
    parser.add_argument("--grace-seconds", required=True, type=float)
    parser.add_argument("--lease-lock", required=True, action="append", type=Path)
    args = parser.parse_args(argv)

    with exclusive_paths(args.lease_lock) as idle:
        if not idle:
            print("compiler cache is in use; preserving its servers")
            return 0
        survivors = reap(
            stale_servers(args.program, args.socket_environment, args.cache_root), args.grace_seconds
        )
    for proc in survivors:
        print(f"pid {proc.pid} survived SIGKILL", file=sys.stderr)
    return 1 if survivors else 0


if __name__ == "__main__":
    raise SystemExit(main())
