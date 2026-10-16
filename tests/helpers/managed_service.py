"""Disposable native supervisor ownership for real service restart acceptance."""

from __future__ import annotations

import json
import os
import plistlib
import re
import signal
import subprocess
import sys
import time
import uuid
from collections.abc import Iterator
from contextlib import AbstractContextManager, contextmanager
from dataclasses import dataclass, field

import psutil

from .constants import ASSETS_DIR, BIN_DIR
from .http_transport import Transport
from .service import ServiceInstance
from .systemd_manager import manager_environment


@dataclass(frozen=True)
class GatewayIdentity:
    port: int
    token: str = field(repr=False)
    service_pid: int
    gateway_pid: int

    def get(self, path: str, *, token: str | None = None) -> tuple[int, object]:
        client = Transport(host="127.0.0.1", port=self.port)
        try:
            status, _, body = client.request("GET", path, headers={
                "Authorization": f"Bearer {self.token if token is None else token}",
            }, timeout=2)
            try:
                payload = json.loads(body)
            except json.JSONDecodeError:
                payload = body.decode()
            return status, payload
        finally:
            client.close()


class ManagedService:
    def __init__(self, service: ServiceInstance) -> None:
        self.service = service
        self.label = f"com.capsem.sdk-restart.{uuid.uuid4().hex}"
        self.domain = f"gui/{os.getuid()}"
        self.processes: dict[int, psutil.Process] = {}

    def remember(self, pid: int) -> None:
        try:
            process = self.processes.get(pid) or psutil.Process(pid)
            if not process.is_running():
                return
            self.processes[pid] = process
            self.processes.update((child.pid, child) for child in process.children(recursive=True))
        except psutil.NoSuchProcess:
            pass

    def command(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(["launchctl", *args], capture_output=True, text=True, timeout=10, check=False)
        if check:
            assert result.returncode == 0, f"launchctl {args}: {result.stderr}"
        return result

    def ready(self, previous: GatewayIdentity | None = None) -> GatewayIdentity:
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            try:
                root = self.service.tmp_dir
                pid = self.pid()
                self.remember(pid)
                identity = GatewayIdentity(
                    port=int((root / "gateway.port").read_text()),
                    token=(root / "gateway.token").read_text().strip(),
                    service_pid=pid,
                    gateway_pid=int((root / "gateway.pid").read_text()),
                )
                changed = previous is None or (
                    identity.service_pid != previous.service_pid
                    and identity.gateway_pid != previous.gateway_pid
                    and identity.token != previous.token
                )
                if changed and identity.get("/vms/list")[0] == 200:
                    return identity
            except (OSError, ValueError, ConnectionError):
                pass
            time.sleep(0.1)
        state = self.startup_state()
        (self.service.tmp_dir / "managed-supervisor-state.txt").write_text(state)
        raise AssertionError(f"managed gateway did not become ready; {state}; evidence: {self.service.tmp_dir}")

    def startup_state(self) -> str:
        result = self.command("list", self.label, check=False)
        return f"launchctl returncode={result.returncode}"

    def pid(self) -> int:
        job = self.command("list", self.label, check=False)
        match = re.search(r'^\t"PID" = ([0-9]+);$', job.stdout, re.MULTILINE)
        if match is None:
            raise ValueError("supervisor has not started the service")
        return int(match[1])

    def retire(self) -> None:
        self.command("bootout", f"{self.domain}/{self.label}", check=False)

    def active(self) -> bool:
        return self.command("list", self.label, check=False).returncode == 0


class SystemdService(ManagedService):
    def __init__(self, service: ServiceInstance) -> None:
        super().__init__(service)
        self.manager_label = f"capsem-sdk-restart-{uuid.uuid4().hex}.service"
        # The private manager has no installed user units; use the product's
        # real unit name so its supervisor ownership query remains truthful.
        self.label = "capsem.service"
        self.environment = dict(os.environ)

    def command(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["systemctl", "--user", *args], env=self.environment,
            capture_output=True, text=True, timeout=10, check=False,
        )
        if check:
            assert result.returncode == 0, f"systemctl {args}: {result.stderr}"
        return result

    def pid(self) -> int:
        result = self.command("show", self.label, "--property=MainPID", "--value", check=False)
        pid = int(result.stdout.strip() or "0")
        if result.returncode or pid == 0:
            raise ValueError("supervisor has not started the service")
        return pid

    def retire(self) -> None:
        self.command("stop", self.label, check=False)

    def active(self) -> bool:
        return self.command("is-active", self.label, check=False).returncode == 0

    def startup_state(self) -> str:
        result = self.command(
            "show", self.label, "--property=ActiveState,SubState,Result,ExecMainStatus,MainPID",
            "--no-pager", check=False,
        )
        return result.stdout.strip() or f"systemctl returncode={result.returncode}"


def service_arguments(service: ServiceInstance) -> list[str]:
    return [
        str(BIN_DIR / "capsem-service"), "--foreground",
        "--uds-path", str(service.uds_path), "--assets-dir", str(service.assets_dir or ASSETS_DIR),
        "--process-binary", str(BIN_DIR / "capsem-process"),
        "--gateway-binary", str(BIN_DIR / "capsem-gateway"), "--gateway-port", "0",
    ]


def service_environment(service: ServiceInstance) -> dict[str, str]:
    (service.home_dir / "corp.toml").write_text("")
    (service.home_dir / "settings.toml").write_text('[settings."app.auto_update"]\nvalue = false\nmodified = "test"\n')
    return {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "CAPSEM_HOME": str(service.home_dir), "CAPSEM_RUN_DIR": str(service.tmp_dir),
        "CAPSEM_CORP_CONFIG": str(service.home_dir / "corp.toml"),
        "CAPSEM_TRAY_HEADLESS": "1",
        "CAPSEM_CREDENTIAL_STORE_PATH": str(service.home_dir / "credential-store.json"),
    }


@contextmanager
def launchd_service(service: ServiceInstance) -> Iterator[ManagedService]:
    owner = ManagedService(service)
    plist = service.home_dir / f"{owner.label}.plist"
    plist.write_bytes(plistlib.dumps({
        "Label": owner.label, "ProgramArguments": service_arguments(service), "KeepAlive": True, "RunAtLoad": True,
        "ThrottleInterval": 1,
        "EnvironmentVariables": service_environment(service),
        "StandardOutPath": str(service.tmp_dir / "managed-service.log"),
        "StandardErrorPath": str(service.tmp_dir / "managed-service.log"),
    }))
    with supervisor_lifetime(service, owner):
        owner.command("bootstrap", owner.domain, str(plist))
        yield owner


@contextmanager
def systemd_service(service: ServiceInstance) -> Iterator[ManagedService]:
    owner = SystemdService(service)
    environment = service_environment(service)
    log = service.tmp_dir / "managed-service.log"
    with (
        supervisor_lifetime(service, owner),
        manager_environment(service.tmp_dir, owner.manager_label, owner.retire) as manager,
    ):
        owner.environment = manager.environment
        result = subprocess.run([
            "systemd-run", "--user", "--collect", f"--unit={owner.label}",
            "--property=Restart=always", "--property=RestartSec=0.1",
            "--property=NoNewPrivileges=yes",
            f"--property=StandardOutput=append:{log}",
            f"--property=StandardError=append:{log}",
            "--property=KillMode=control-group", "--property=TimeoutStopSec=15",
            *(f"--setenv={key}={value}" for key, value in environment.items()),
            *service_arguments(service),
        ], env=manager.environment, capture_output=True, text=True, timeout=10, check=False)
        assert result.returncode == 0, result.stdout + result.stderr
        yield owner


def managed_service(service: ServiceInstance) -> AbstractContextManager[ManagedService]:
    if sys.platform == "darwin":
        return launchd_service(service)
    if sys.platform == "linux":
        return systemd_service(service)
    raise RuntimeError(f"unsupported native supervisor: {sys.platform}")


@contextmanager
def supervisor_lifetime(service: ServiceInstance, owner: ManagedService) -> Iterator[None]:
    previous_handler = signal.getsignal(signal.SIGTERM)

    def terminate(_signal: int, _frame: object) -> None:
        raise SystemExit("managed restart fixture interrupted")

    signal.signal(signal.SIGTERM, terminate)
    try:
        yield
    finally:
        try:
            for pid in list(owner.processes):
                owner.remember(pid)
            owner.retire()
            deadline = time.monotonic() + 15
            while owner.active():
                assert time.monotonic() < deadline, "fixture job leaked"
                time.sleep(0.05)
            _, alive = psutil.wait_procs(list(owner.processes.values()), timeout=15)
            for process in alive:
                process.kill()
                process.wait(timeout=5)
            assert not alive, "fixture companions required forced cleanup"
        finally:
            signal.signal(signal.SIGTERM, previous_handler)
            service.stop()
