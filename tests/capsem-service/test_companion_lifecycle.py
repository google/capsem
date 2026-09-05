"""Companion (capsem-gateway, capsem-tray) lifecycle and singleton tests.

These tests lock in the defense-in-depth contract between the service and its
companions:

    1. Companions REFUSE to run standalone. Launched without --parent-pid, or
       with a --parent-pid that is not actually our PID, they exit 0 within
       one watch-tick (~500 ms).
    2. Companions are SINGLETONS. Second spawns (same lock path) exit 0.
       Hammering N parallel spawns yields exactly one live companion.
    3. Companions DIE WITH THE PARENT. When the service is SIGKILL'd the
       companions exit within a bounded deadline -- no orphans left under
       PID 1. This works with SIGKILL because the kernel reparents orphans
       and the companion's watcher detects the PPID change.

Together these prevent the orphan-accumulation class of bug that used to
make `just test -n 4` flake: interrupted or killed test runs can no longer
leak trays/gateways that interfere with subsequent runs.
"""

from __future__ import annotations

import contextlib
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

import psutil
import pytest
from helpers.http_transport import Transport
from helpers.service import ServiceInstance
from helpers.sign import sign_binary

pytestmark = pytest.mark.integration


PROJECT_ROOT = Path(__file__).parent.parent.parent
TRAY_BIN = PROJECT_ROOT / "cache/target/cargo/debug/capsem-tray"
GATEWAY_BIN = PROJECT_ROOT / "cache/target/cargo/debug/capsem-gateway"

# Parent-watch poll interval is 500ms in capsem-guard; give a generous factor
# for loaded CI while still catching real regressions.
WATCH_DEADLINE_SECS = 5.0


def _sign():
    sign_binary(TRAY_BIN)
    sign_binary(GATEWAY_BIN)


def _spawn(binary: Path, *args: str, env_extra: dict | None = None) -> subprocess.Popen:
    env = os.environ.copy()
    env.setdefault("RUST_LOG", "info")
    # Run trays without a menu-bar icon. Companion-lifecycle tests exercise
    # parent-watch and the singleton lock -- they don't need UI, and without
    # this every test run flashes the user's menu bar.
    env.setdefault("CAPSEM_TRAY_HEADLESS", "1")
    if env_extra:
        env.update(env_extra)
    return subprocess.Popen(
        [str(binary), *args],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _spawn_under_parent(
    binary: Path,
    parent_pid_file: Path,
    child_pid_file: Path,
    extra_args: list[str],
    env_extra: dict | None = None,
) -> subprocess.Popen:
    """Run `binary` as a child of a shell process.

    Returns the Popen handle for the shell; the companion is spawned by the
    shell with PPID == shell's PID, which the shell writes to
    parent_pid_file. The companion's own PID is written to child_pid_file.
    Signalling the shell SIGKILL simulates ungraceful parent death.

    This matches production: the service is a long-lived parent process that
    spawns companions; the test shell stands in for the service.
    """
    args_q = " ".join(f'"{a}"' for a in extra_args)
    # Parent shell: prints its own PID, forks the companion with --parent-pid $$,
    # writes the companion's pid, then sleeps forever to stay as parent.
    script = (
        f'echo "$$" > "{parent_pid_file}"\n'
        f'"{binary}" --parent-pid "$$" {args_q} &\n'
        f'echo "$!" > "{child_pid_file}"\n'
        f"exec sleep 600\n"
    )
    env = os.environ.copy()
    env.setdefault("RUST_LOG", "info")
    env.setdefault("CAPSEM_TRAY_HEADLESS", "1")
    if env_extra:
        env.update(env_extra)
    return subprocess.Popen(
        ["bash", "-c", script],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _read_pid(path: Path, deadline: float) -> int:
    end = time.time() + deadline
    while time.time() < end:
        try:
            content = path.read_text().strip()
            if content:
                return int(content)
        except (FileNotFoundError, ValueError):
            pass
        time.sleep(0.05)
    raise TimeoutError(f"pid file {path} never populated")


def _wait_exit(proc: subprocess.Popen, deadline: float) -> int | None:
    """Poll for exit; returns exit code or None if still alive at deadline."""
    end = time.time() + deadline
    while time.time() < end:
        if proc.poll() is not None:
            return proc.returncode
        time.sleep(0.05)
    return None


def _wait_pid_exit(pid: int, deadline: float) -> bool:
    """Wait until the given PID is gone. Returns True if exited in deadline."""
    end = time.time() + deadline
    while time.time() < end:
        if not _pid_alive(pid):
            return True
        time.sleep(0.05)
    return False


class TestCompanionRefusesStandalone:
    """A companion invoked without a valid parent must exit 0 quickly.

    This is rule A: 'standalone == no-op'. Users running `./capsem-tray` by
    hand get a silent, clean exit -- no orphan running in the menu bar.
    """

    def test_tray_without_parent_pid_exits(self):
        _sign()
        with tempfile.TemporaryDirectory() as td:
            env = {"CAPSEM_RUN_DIR": td}
            proc = _spawn(TRAY_BIN, env_extra=env)
            rc = _wait_exit(proc, WATCH_DEADLINE_SECS)
            assert rc == 0, f"tray without --parent-pid must exit 0, got {rc}"

    def test_tray_with_non_parent_pid_exits(self):
        _sign()
        # Pass a PID that exists but is NOT our spawned tray's actual parent
        # (our own parent works: it's alive, but tray's PPID is us, not it).
        wrong_pid = os.getppid()
        with tempfile.TemporaryDirectory() as td:
            env = {"CAPSEM_RUN_DIR": td}
            proc = _spawn(TRAY_BIN, "--parent-pid", str(wrong_pid), env_extra=env)
            rc = _wait_exit(proc, WATCH_DEADLINE_SECS)
            assert rc == 0, f"tray with wrong --parent-pid must exit 0, got {rc}"

    def test_gateway_without_parent_pid_exits(self):
        _sign()
        with tempfile.TemporaryDirectory() as td:
            env = {"CAPSEM_RUN_DIR": td}
            proc = _spawn(GATEWAY_BIN, "--port", "0", env_extra=env)
            rc = _wait_exit(proc, WATCH_DEADLINE_SECS)
            assert rc == 0, f"gateway without --parent-pid must exit 0, got {rc}"

    def test_gateway_with_non_parent_pid_exits(self):
        _sign()
        wrong_pid = os.getppid()
        with tempfile.TemporaryDirectory() as td:
            env = {"CAPSEM_RUN_DIR": td}
            proc = _spawn(
                GATEWAY_BIN,
                "--port",
                "0",
                "--parent-pid",
                str(wrong_pid),
                env_extra=env,
            )
            rc = _wait_exit(proc, WATCH_DEADLINE_SECS)
            assert rc == 0, f"gateway with wrong --parent-pid must exit 0, got {rc}"


class TestCompanionSingleton:
    """Only one instance of a companion can hold the singleton lock."""

    def test_tray_second_spawn_exits(self):
        """Two trays with the same parent & lock: second exits 0, first stays."""
        _sign()
        with tempfile.TemporaryDirectory() as td:
            lock = Path(td) / "tray.lock"
            parent_pid_file = Path(td) / "parent.pid"
            first_child_pid_file = Path(td) / "first.pid"
            second_child_pid_file = Path(td) / "second.pid"

            first_parent = _spawn_under_parent(
                TRAY_BIN,
                parent_pid_file,
                first_child_pid_file,
                ["--lock-path", str(lock)],
                env_extra={"CAPSEM_RUN_DIR": td},
            )
            try:
                first_child_pid = _read_pid(first_child_pid_file, 3.0)
                # Give the first tray time to acquire the lock.
                time.sleep(0.4)
                assert _pid_alive(first_child_pid), (
                    f"first tray {first_child_pid} must still be running"
                )

                # Spawn the second tray under a DIFFERENT parent shell, using
                # the SAME lock path. The second tray must bounce on the
                # singleton check and exit 0.
                second_parent = _spawn_under_parent(
                    TRAY_BIN,
                    Path(td) / "parent2.pid",
                    second_child_pid_file,
                    ["--lock-path", str(lock)],
                    env_extra={"CAPSEM_RUN_DIR": td},
                )
                try:
                    second_child_pid = _read_pid(second_child_pid_file, 3.0)
                    assert _wait_pid_exit(second_child_pid, WATCH_DEADLINE_SECS), (
                        "second tray must exit (singleton)"
                    )
                    assert _pid_alive(first_child_pid), (
                        "first tray must still be running after second exits"
                    )
                finally:
                    second_parent.kill()
                    second_parent.wait(timeout=5)
            finally:
                first_parent.kill()
                first_parent.wait(timeout=5)

    def test_tray_hammer_20_parallel_yields_one_live(self):
        """Spawn 20 trays concurrently against one lock path. Exactly one
        stays; the other 19 exit 0."""
        _sign()
        with tempfile.TemporaryDirectory() as td:
            lock = Path(td) / "tray.lock"

            parents = []
            child_pid_files = []
            for i in range(20):
                child_pid_file = Path(td) / f"child.{i}.pid"
                child_pid_files.append(child_pid_file)
                parents.append(
                    _spawn_under_parent(
                        TRAY_BIN,
                        Path(td) / f"parent.{i}.pid",
                        child_pid_file,
                        ["--lock-path", str(lock)],
                        env_extra={"CAPSEM_RUN_DIR": td},
                    )
                )

            try:
                # Wait for all child PIDs to be written.
                child_pids = []
                for f in child_pid_files:
                    try:
                        child_pids.append(_read_pid(f, 5.0))
                    except TimeoutError:
                        pytest.fail(f"child pid file {f} never populated")

                # Let the singleton-losing trays exit.
                time.sleep(2.0)

                alive = [pid for pid in child_pids if _pid_alive(pid)]
                assert len(alive) == 1, (
                    f"exactly one tray should remain alive, got {len(alive)} "
                    f"alive out of {len(child_pids)}"
                )
            finally:
                for p in parents:
                    p.kill()
                for p in parents:
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        p.wait(timeout=5)

    def test_gateway_second_spawn_exits(self):
        _sign()
        with tempfile.TemporaryDirectory() as td:
            lock = Path(td) / "gateway.lock"
            first_child_pid_file = Path(td) / "first.pid"
            second_child_pid_file = Path(td) / "second.pid"

            first_parent = _spawn_under_parent(
                GATEWAY_BIN,
                Path(td) / "parent1.pid",
                first_child_pid_file,
                ["--port", "0", "--lock-path", str(lock)],
                env_extra={"CAPSEM_RUN_DIR": td},
            )
            try:
                first_child_pid = _read_pid(first_child_pid_file, 3.0)
                time.sleep(0.4)
                assert _pid_alive(first_child_pid), (
                    f"first gateway {first_child_pid} must still be running"
                )

                second_parent = _spawn_under_parent(
                    GATEWAY_BIN,
                    Path(td) / "parent2.pid",
                    second_child_pid_file,
                    ["--port", "0", "--lock-path", str(lock)],
                    env_extra={"CAPSEM_RUN_DIR": td},
                )
                try:
                    second_child_pid = _read_pid(second_child_pid_file, 3.0)
                    assert _wait_pid_exit(second_child_pid, WATCH_DEADLINE_SECS), (
                        "second gateway must exit (singleton)"
                    )
                finally:
                    second_parent.kill()
                    second_parent.wait(timeout=5)
            finally:
                first_parent.kill()
                first_parent.wait(timeout=5)


class TestCompanionDiesWithParent:
    """A running companion must exit within a bounded deadline when its
    parent dies -- including ungraceful SIGKILL."""

    def test_tray_exits_when_parent_sigkilled(self):
        _sign()
        with tempfile.TemporaryDirectory() as td:
            parent_pid_file = Path(td) / "parent.pid"
            child_pid_file = Path(td) / "child.pid"
            parent = _spawn_under_parent(
                TRAY_BIN,
                parent_pid_file,
                child_pid_file,
                ["--lock-path", str(Path(td) / "tray.lock")],
                env_extra={"CAPSEM_RUN_DIR": td},
            )
            try:
                parent_pid = _read_pid(parent_pid_file, 3.0)
                child_pid = _read_pid(child_pid_file, 3.0)
                # Prove the tray is running BEFORE killing the parent.
                time.sleep(0.5)
                assert _pid_alive(child_pid), (
                    f"tray {child_pid} must be running prior to parent kill"
                )
                assert _pid_alive(parent_pid), f"parent shell {parent_pid} must be running"

                os.kill(parent_pid, signal.SIGKILL)

                assert _wait_pid_exit(child_pid, WATCH_DEADLINE_SECS), (
                    f"tray {child_pid} must exit after parent {parent_pid} "
                    "SIGKILL -- orphan regression"
                )
            finally:
                with contextlib.suppress(ProcessLookupError):
                    parent.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    parent.wait(timeout=5)

    def test_gateway_exits_when_parent_sigkilled(self):
        _sign()
        with tempfile.TemporaryDirectory() as td:
            parent_pid_file = Path(td) / "parent.pid"
            child_pid_file = Path(td) / "child.pid"
            parent = _spawn_under_parent(
                GATEWAY_BIN,
                parent_pid_file,
                child_pid_file,
                ["--port", "0", "--lock-path", str(Path(td) / "gateway.lock")],
                env_extra={"CAPSEM_RUN_DIR": td},
            )
            try:
                parent_pid = _read_pid(parent_pid_file, 3.0)
                child_pid = _read_pid(child_pid_file, 3.0)
                time.sleep(0.5)
                assert _pid_alive(child_pid), (
                    f"gateway {child_pid} must be running prior to parent kill"
                )

                os.kill(parent_pid, signal.SIGKILL)

                assert _wait_pid_exit(child_pid, WATCH_DEADLINE_SECS), (
                    f"gateway {child_pid} must exit after parent SIGKILL"
                )
            finally:
                with contextlib.suppress(ProcessLookupError):
                    parent.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    parent.wait(timeout=5)


class TestServiceSigkillReapsAllCompanions:
    """End-to-end: SIGKILL the service and verify gateway + tray are gone
    within the watch deadline. This is the regression guard for the orphan
    accumulation that used to flake `just test -n 4`."""

    def test_sigkill_service_kills_companions(self):
        svc = ServiceInstance()
        svc.start()
        service_pid = svc.proc.pid
        try:
            # Give companions a moment to spawn and register their guards.
            time.sleep(1.0)

            # Collect companion PIDs as direct children of the service.
            children = _list_direct_children(service_pid)
            # Service always spawns at least gateway; on macOS also tray.
            assert children, f"service {service_pid} must have spawned companion children; got none"

            # SIGKILL the service (no graceful shutdown path runs).
            os.kill(service_pid, signal.SIGKILL)
            svc.proc.wait()

            deadline = time.time() + WATCH_DEADLINE_SECS
            stragglers: list[int] = []
            while time.time() < deadline:
                stragglers = [pid for pid in children if _pid_alive(pid)]
                if not stragglers:
                    break
                time.sleep(0.1)

            assert not stragglers, (
                f"companions still alive after service SIGKILL: {stragglers}. "
                "Regression: capsem-guard parent-watch not enforced."
            )
        finally:
            # Avoid the normal shutdown path (service is already dead); just
            # clean the tmp dir.
            svc.proc = None
            svc.stop()


class TestServiceSigtermReapsCompanionsPromptly:
    """`just shell` / `just ui` / `just run-service` all invoke `_ensure-service`,
    which SIGTERMs any prior dev service and sleeps 500ms before spawning a
    new one. The new gateway binds to the same TCP port (19222 by default),
    so the OLD gateway MUST be gone within that 500ms budget -- otherwise
    the new gateway hits EADDRINUSE, exits, and the dev service runs with
    no gateway. Users see "ws connected" briefly (served by the orphan),
    then the orphan's parent-watch fires and the gateway is gone for good.

    Contract: after SIGTERM to capsem-service (no VMs running), all
    companions exit within 300 ms -- well under the 500 ms `_ensure-service`
    waits. This is stricter than the SIGKILL case because graceful shutdown
    runs and can actively kill children.
    """

    SIGTERM_COMPANION_BUDGET_SECS = 0.3

    def test_sigterm_service_kills_companions_within_budget(self):
        svc = ServiceInstance()
        svc.start()
        service_pid = svc.proc.pid
        try:
            # Let companions spawn + register guards.
            time.sleep(1.0)

            children = _list_direct_children(service_pid)
            assert children, f"service {service_pid} must have spawned companion children; got none"

            # SIGTERM the service (default pkill signal -- exactly what
            # `_ensure-service` sends via `pkill -f capsem-service.*--foreground`).
            os.kill(service_pid, signal.SIGTERM)

            # Assert companions are dead within the budget. We do NOT wait
            # for the service to exit here: the contract is about companions
            # dying fast enough that the next `_ensure-service` can spawn a
            # new gateway on the same TCP port without racing.
            deadline = time.time() + self.SIGTERM_COMPANION_BUDGET_SECS
            stragglers: list[int] = []
            while time.time() < deadline:
                stragglers = [pid for pid in children if _pid_alive(pid)]
                if not stragglers:
                    break
                time.sleep(0.01)

            assert not stragglers, (
                f"companions still alive {self.SIGTERM_COMPANION_BUDGET_SECS}s "
                f"after service SIGTERM: {stragglers}. This races with "
                "`_ensure-service`'s 500ms sleep -- when `just shell`/`just ui` "
                "are re-invoked, the new gateway fails to bind :19222 because "
                "the old gateway is still alive."
            )
        finally:
            # Service may still be alive (we sent SIGTERM but didn't wait).
            # Let stop() handle termination + cleanup.
            svc.stop()


class TestCompanionsDieFastAfterServiceSigkill:
    """`just ui` / `just shell` invoke `_ensure-service`, which SIGTERMs any
    prior service and sleeps exactly 500 ms before spawning the new one.
    If the prior service was instead SIGKILL'd out-of-band (crash, user
    `kill -9`, OOM) graceful shutdown does NOT run, so companions only die
    via capsem-guard's parent-watch -- a polling loop.

    Contract: companion exit latency after an ungraceful parent death must
    comfortably fit inside the `_ensure-service` restart budget, otherwise
    port 19222 is still held by the orphan gateway when the new service
    tries to spawn its own gateway. We allow 300 ms, i.e. roughly half of
    the budget, to leave headroom for CI jitter.

    The existing `TestServiceSigkillReapsAllCompanions` uses a 5 s
    deadline -- useful for catching the "companions leak forever" bug
    but far too loose to catch the "parent-watch is too slow for the
    restart budget" bug.
    """

    SIGKILL_COMPANION_BUDGET_SECS = 0.3

    def test_sigkill_service_kills_companions_within_restart_budget(self):
        svc = ServiceInstance()
        svc.start()
        service_pid = svc.proc.pid
        try:
            time.sleep(1.0)  # let companions spawn + register guards
            children = _list_direct_children(service_pid)
            assert children, f"service {service_pid} must have spawned companion children"

            # SIGKILL: no graceful_shutdown runs, companions rely entirely
            # on parent-watch in capsem-guard.
            os.kill(service_pid, signal.SIGKILL)
            svc.proc.wait()

            deadline = time.time() + self.SIGKILL_COMPANION_BUDGET_SECS
            stragglers: list[int] = []
            while time.time() < deadline:
                stragglers = [pid for pid in children if _pid_alive(pid)]
                if not stragglers:
                    break
                time.sleep(0.01)

            assert not stragglers, (
                f"companions still alive {self.SIGKILL_COMPANION_BUDGET_SECS}s "
                f"after service SIGKILL: {stragglers}. "
                "parent-watch poll interval is too long for the "
                "`_ensure-service` 500ms restart budget -- the new gateway "
                "will race the orphan on port 19222."
            )
        finally:
            svc.proc = None
            svc.stop()


class TestServiceShutdownIsFastWithoutVMs:
    """`kill_all_vm_processes` runs inside the service's graceful-shutdown
    path. It currently performs a 500 ms `thread::sleep` between SIGTERM and
    SIGKILL of VM processes -- *unconditionally*, even when zero VMs are
    running. Because `_ensure-service` waits exactly 500 ms between killing
    the old service and spawning the new one, any additional shutdown latency
    pushes the overlap window wider and reintroduces the gateway-orphan race
    we fixed in `TestServiceSigtermReapsCompanionsPromptly`.

    Contract: a SIGTERM-to-exit cycle on a service with no VMs must complete
    well under the 500 ms `_ensure-service` budget. 300 ms gives us a
    generous CI headroom while still catching the unconditional-sleep bug.
    """

    NO_VM_SHUTDOWN_BUDGET_SECS = 0.3

    def test_shutdown_completes_within_budget_when_no_vms(self):
        svc = ServiceInstance()
        svc.start()
        try:
            # Let the service settle (companions spawn, startup finishes).
            time.sleep(1.0)

            start = time.time()
            os.kill(svc.proc.pid, signal.SIGTERM)
            try:
                svc.proc.wait(timeout=self.NO_VM_SHUTDOWN_BUDGET_SECS + 1.0)
            except subprocess.TimeoutExpired:
                svc.proc.kill()
                svc.proc.wait(timeout=5)
                pytest.fail(
                    "service did not exit within "
                    f"{self.NO_VM_SHUTDOWN_BUDGET_SECS + 1.0}s of SIGTERM"
                )
            elapsed = time.time() - start

            assert elapsed < self.NO_VM_SHUTDOWN_BUDGET_SECS, (
                f"service shutdown took {elapsed:.2f}s with zero VMs running "
                f"(budget: {self.NO_VM_SHUTDOWN_BUDGET_SECS}s). "
                "kill_all_vm_processes must skip its 500ms SIGTERM->SIGKILL "
                "grace sleep when the VM list is empty -- `_ensure-service` "
                "only waits 500ms before respawning."
            )
        finally:
            # proc already reaped above; suppress terminate() on dead handle.
            svc.proc = None
            svc.stop()


class TestServiceRestartSequenceKeepsGatewayHealthy:
    """End-to-end contract for what `just ui` / `just shell` do: kill the
    running dev service, wait 500 ms, spawn a fresh one on the SAME run_dir
    and gateway port. The new service's gateway must bind the port (old
    gateway fully gone) and respond to /health, and be a direct child of
    the new service (not an orphan).

    Regression: prior to the shutdown-order fix, the old service's
    graceful shutdown killed VMs *before* companions. With a 500 ms
    VM-kill grace period, the old gateway stayed alive past the 500 ms
    `_ensure-service` window, leaving the new gateway to race -- and often
    lose -- for `bind(:19222)`.
    """

    ENSURE_SERVICE_SLEEP_SECS = 0.5

    def test_restart_with_same_run_dir_and_port(self):
        svc = ServiceInstance()
        try:
            svc.start()
            # Service UDS readiness precedes the companion publishing its port.
            svc.gateway_port = _wait_for_gateway_port_file(
                svc.tmp_dir, None, timeout=WATCH_DEADLINE_SECS,
            )
            children_a = _list_direct_children(svc.proc.pid)
            assert children_a, "service must have companion children before restart"

            os.kill(svc.proc.pid, signal.SIGTERM)
            time.sleep(self.ENSURE_SERVICE_SLEEP_SECS)
            svc.proc.wait(timeout=5)
            survivors = [pid for pid in children_a if _pid_alive(pid)]
            assert not survivors, (
                f"companions survived the restart budget "
                f"({self.ENSURE_SERVICE_SLEEP_SECS}s): {survivors}"
            )
            svc.stop(cleanup=False)

            # Reuse the home, run directory and actual TCP port. A surviving
            # gateway must fail this restart instead of getting a fresh port.
            svc.start()
            _wait_for_gateway_port_file(
                svc.tmp_dir, svc.gateway_port, timeout=WATCH_DEADLINE_SECS,
            )
            children_b = _list_direct_children(svc.proc.pid)
            assert children_b, "service must spawn companion children on restart"
            assert not set(children_a).intersection(children_b), (
                "the restarted service must spawn fresh companions"
            )
        finally:
            # Startup assertions need the same cleanup as shutdown assertions.
            svc.stop()


class TestRapidServiceRestartIsRobust:
    """Stress test: many `just ui` / `just shell` invocations in a row, all
    using the SAME fixed TCP port for the gateway. Each restart must
    produce a healthy gateway bound to that port. This exercises the
    orphan-gateway race under load -- a single flake out of N iterations
    is a bug.

    Repetition exposes poll races, kernel scheduling jitter and port reuse
    failures that a single fixed-port restart can miss.
    """

    ITERATIONS = 6
    # Budget between signalling the old service and requiring its
    # companions to be dead. `_ensure-service` sleeps 500 ms, but we keep
    # 300 ms of headroom so the test surfaces a regression (e.g. poll
    # interval regressed to 500 ms) instead of sitting right on the edge.
    COMPANION_DEATH_BUDGET_SECS = 0.3

    def test_six_rapid_restarts_on_fixed_port_all_produce_healthy_gateway(self):
        _sign()
        # Use a high, process-specific port to avoid colliding with any
        # other test or the real :19222 that the user may have bound.
        port = 30000 + (os.getpid() % 5000)
        shared_tmp = Path(tempfile.mkdtemp(prefix="capsem-rapid-restart-"))
        spawned: list[subprocess.Popen] = []
        try:
            prev_proc: subprocess.Popen | None = None
            prev_children: list[int] = []
            for i in range(self.ITERATIONS):
                if prev_proc is not None:
                    # `_ensure-service` sequence: SIGTERM, then sleep before
                    # spawning the next one. After our fixes the graceful
                    # shutdown path completes fast; and if it is skipped
                    # (SIGKILL case covered below) parent-watch covers the
                    # gap. Either way, companions must die well within the
                    # 300 ms headroom.
                    os.kill(prev_proc.pid, signal.SIGTERM)
                    prev_proc.wait(timeout=5)
                    deadline = time.time() + self.COMPANION_DEATH_BUDGET_SECS
                    for pid in prev_children:
                        while time.time() < deadline and _pid_alive(pid):
                            time.sleep(0.01)
                        assert not _pid_alive(pid), (
                            f"iteration {i}: companion {pid} from previous "
                            f"service still alive "
                            f"{self.COMPANION_DEATH_BUDGET_SECS}s after SIGTERM"
                        )

                proc = _spawn_service_on_fixed_port(shared_tmp, port)
                spawned.append(proc)
                try:
                    _wait_for_gateway_port_file(shared_tmp, port, timeout=5.0)
                except TimeoutError as e:
                    proc.kill()
                    proc.wait(timeout=5)
                    pytest.fail(
                        f"iteration {i}: {e}. Likely the new gateway failed "
                        "to bind -- orphan still holding the port."
                    )
                # Actively verify gateway is listening on the port we pinned.
                assert _port_is_listening(port), (
                    f"iteration {i}: nothing listening on :{port} after "
                    "service startup; new gateway did not bind."
                )
                children = _list_direct_children(proc.pid)
                assert children, f"iteration {i}: service has no companion children"
                prev_proc = proc
                prev_children = children

            # Clean up the final service.
            if prev_proc is not None:
                os.kill(prev_proc.pid, signal.SIGTERM)
                prev_proc.wait(timeout=5)
        finally:
            # An assertion may interrupt an iteration before its service is
            # assigned to prev_proc. Reap every survivor so this stress test
            # cannot leak a service into unrelated tests.
            for proc in spawned:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=5)
                log_file = getattr(proc, "_log_file", None)
                if log_file is not None and not log_file.closed:
                    log_file.close()
            shutil.rmtree(shared_tmp, ignore_errors=True)


def _spawn_service_on_fixed_port(
    tmp_dir: Path,
    gateway_port: int,
) -> subprocess.Popen:
    """Spawn a capsem-service with a pinned gateway port and shared run_dir.

    Mirrors ServiceInstance.start() but lets us pin `--gateway-port` so
    consecutive services collide on the same port (the real `just ui`
    scenario). Returns the Popen handle; caller owns shutdown.

    The log-file handle is stashed on the returned proc as
    `proc._log_file` so the caller can close it after `proc.wait()` --
    Popen does not close file objects passed as stdout/stderr, and
    filterwarnings=error promotes the leaked fd to a hard failure.
    """
    from helpers.service import (
        ASSETS_DIR,
        GATEWAY_BINARY,
        PROCESS_BINARY,
        SERVICE_BINARY,
        TRAY_BINARY,
        materialize_test_profiles,
    )

    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    assets_dir = ASSETS_DIR / arch
    uds_path = tmp_dir / f"service-{uuid.uuid4().hex[:8]}.sock"
    log_path = tmp_dir / f"service-{uuid.uuid4().hex[:4]}.log"
    env = os.environ.copy()
    env["RUST_LOG"] = "info"
    env["CAPSEM_RUN_DIR"] = str(tmp_dir)
    env["CAPSEM_HOME"] = str(tmp_dir)
    env["CAPSEM_PROFILES_DIR"] = str(materialize_test_profiles(tmp_dir))
    env["HOME"] = str(tmp_dir)
    env["CAPSEM_TRAY_HEADLESS"] = "1"
    log_file = open(log_path, "w")  # noqa: SIM115 -- handed to Popen; must outlive this statement
    proc = subprocess.Popen(
        [
            str(SERVICE_BINARY),
            "--uds-path",
            str(uds_path),
            "--assets-dir",
            str(assets_dir),
            "--process-binary",
            str(PROCESS_BINARY),
            "--gateway-binary",
            str(GATEWAY_BINARY),
            "--gateway-port",
            str(gateway_port),
            "--tray-binary",
            str(TRAY_BINARY),
            "--foreground",
        ],
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
    )
    proc._log_file = log_file  # type: ignore[attr-defined]
    # Wait for service UDS to accept.
    start = time.time()
    while time.time() - start < 15:
        if uds_path.exists():
            probe = Transport(socket_path=str(uds_path))
            try:
                probe.request("GET", "/vms/list", timeout=2)
                return proc
            except Exception:
                pass
            finally:
                probe.close()
        time.sleep(0.1)
    proc.kill()
    proc.wait(timeout=5)
    log_file.close()
    if log_path.exists():
        print(f"\n--- SERVICE LOG ---\n{log_path.read_text()}\n---", file=sys.stderr)
    raise RuntimeError("service never accepted UDS connections")


def _wait_for_gateway_port_file(run_dir: Path, expected_port: int | None, timeout: float) -> int:
    """Wait for the published port and a healthy gateway, optionally pinning it."""
    path = run_dir / "gateway.port"
    end = time.monotonic() + timeout
    last_seen = None
    while time.monotonic() < end:
        if path.exists():
            try:
                last_seen = path.read_text().strip()
                port = int(last_seen)
                if 0 < port < 65536 and (expected_port is None or port == expected_port):
                    probe = Transport(host="127.0.0.1", port=port)
                    try:
                        status, _, body = probe.request("GET", "/health", timeout=0.5)
                        if status == 200 and b"ok" in body.lower():
                            return port
                    finally:
                        probe.close()
            except (OSError, ValueError):
                pass
        time.sleep(0.05)
    raise TimeoutError(
        f"gateway.port at {path} never reported {expected_port} "
        f"(last seen: {last_seen!r}) within {timeout}s"
    )


def _port_is_listening(port: int) -> bool:
    """Check whether anything is listening on 127.0.0.1:<port>."""
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.settimeout(0.5)
        try:
            s.connect(("127.0.0.1", port))
            return True
        except (TimeoutError, ConnectionRefusedError):
            return False


def _list_direct_children(ppid: int) -> list[int]:
    """Return PIDs whose PPID == ppid, using `pgrep -P`."""
    try:
        out = subprocess.check_output(["pgrep", "-P", str(ppid)], text=True)
    except subprocess.CalledProcessError:
        return []
    return [int(line.strip()) for line in out.splitlines() if line.strip()]


def _pid_alive(pid: int) -> bool:
    """True iff pid belongs to a running, non-zombie process.

    `os.kill(pid, 0)` would also return True for zombies (SIGKILL'd but not
    yet reaped). Zombies are "dead" for our purposes -- a zombie tray has
    already exited, so we must not count it as alive.

    Through `pidfiles.running`, which is the production answer to this exact
    question. This shelled `ps` instead, and `/bin/ps` is setuid root: macOS
    forbids a sandboxed process from exec'ing one, whatever its profile
    permits. So this passed everywhere except inside the gate, where it failed
    with `PermissionError: Operation not permitted: 'ps'` -- and it failed the
    same way in two consecutive runs while passing every way it could be run
    by hand.

    The gate had already stopped using `ps` for this; the test kept its own
    copy, so the fix never reached it. A second implementation of a question
    production already answers is a second thing to get wrong.
    """
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import pidfiles

    root = Path(__file__).resolve().parents[2]
    return pidfiles.running(pid, gate_config.load(root).pidfiles)


class TestServiceTrayStaysOffTheMenuBar:
    """A suite run must not put icons in the developer's menu bar.

    Omitting --tray-binary does not prevent the tray: the service falls back
    to `find_sibling_binary("capsem-tray")`, a fallback the CLI's own
    auto-start depends on because it passes no companion paths at all. And the
    tray's singleton lock lives at `$CAPSEM_RUN_DIR/tray.lock`, which every
    ServiceInstance points at its own temp dir, so the locks never dedupe --
    one live tray per service, each building a real NSStatusItem.

    CAPSEM_TRAY_HEADLESS is what actually keeps the menu bar quiet. It drops
    only the icon: the companion still starts, holds its guard and lock, and
    is reaped with the service, so the coverage above stays intact.
    """

    def test_suite_env_runs_trays_headless(self):
        assert os.environ.get("CAPSEM_TRAY_HEADLESS"), (
            "tests/conftest.py must mark the whole suite headless before any "
            "service starts; without it every ServiceInstance adds a menu bar "
            "icon on macOS"
        )

    def test_service_passes_headless_through_to_its_tray(self):
        """The setting is useless if it does not reach the spawned companion."""
        svc = ServiceInstance()
        svc.start()
        try:
            time.sleep(1.0)
            if sys.platform != "darwin":
                pytest.skip("the tray is macOS-only")
            trays = [
                psutil.Process(pid)
                for pid in _list_direct_children(svc.proc.pid)
                if "capsem-tray" in " ".join(psutil.Process(pid).cmdline())
            ]
            assert trays, (
                "expected the service to spawn a tray via sibling fallback; if "
                "this stops being true the headless rail is guarding nothing"
            )
            for tray in trays:
                assert "CAPSEM_TRAY_HEADLESS" in tray.environ(), (
                    f"tray {tray.pid} did not inherit CAPSEM_TRAY_HEADLESS, so "
                    "it built a real menu bar icon"
                )
        finally:
            svc.stop()
