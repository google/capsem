"""Disposable launchd ownership for real service restart acceptance."""

from __future__ import annotations

import json
import os
import plistlib
import re
import signal
import subprocess
import time
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field

import psutil

from .constants import ASSETS_DIR, BIN_DIR
from .http_transport import Transport
from .service import ServiceInstance


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
                job = self.command("list", self.label, check=False)
                match = re.search(r'^\t"PID" = ([0-9]+);$', job.stdout, re.MULTILINE)
                if match is None:
                    raise ValueError("supervisor has not started the service")
                self.remember(int(match[1]))
                identity = GatewayIdentity(
                    port=int((root / "gateway.port").read_text()),
                    token=(root / "gateway.token").read_text().strip(),
                    service_pid=int(match[1]),
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
        raise AssertionError(f"managed gateway did not become ready; evidence: {self.service.tmp_dir}")


@contextmanager
def launchd_service(service: ServiceInstance) -> Iterator[ManagedService]:
    owner = ManagedService(service)
    assert service.profiles_dir is not None
    (service.home_dir / "corp.toml").write_text("")
    (service.home_dir / "settings.toml").write_text('[settings."app.auto_update"]\nvalue = false\nmodified = "test"\n')
    plist = service.home_dir / f"{owner.label}.plist"
    args = [
        str(BIN_DIR / "capsem-service"), "--foreground",
        "--uds-path", str(service.uds_path), "--assets-dir", str(service.assets_dir or ASSETS_DIR),
        "--process-binary", str(BIN_DIR / "capsem-process"),
        "--gateway-binary", str(BIN_DIR / "capsem-gateway"), "--gateway-port", "0",
    ]
    plist.write_bytes(plistlib.dumps({
        "Label": owner.label, "ProgramArguments": args, "KeepAlive": True, "RunAtLoad": True,
        "ThrottleInterval": 1,
        "EnvironmentVariables": {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "CAPSEM_HOME": str(service.home_dir), "CAPSEM_RUN_DIR": str(service.tmp_dir),
            "CAPSEM_CORP_CONFIG": str(service.home_dir / "corp.toml"),
            "CAPSEM_PROFILES_DIR": str(service.profiles_dir), "CAPSEM_TRAY_HEADLESS": "1",
            "CAPSEM_CREDENTIAL_STORE_PATH": str(service.home_dir / "credential-store.json"),
        },
        "StandardOutPath": str(service.tmp_dir / "managed-service.log"),
        "StandardErrorPath": str(service.tmp_dir / "managed-service.log"),
    }))
    previous_handler = signal.getsignal(signal.SIGTERM)

    def terminate(_signal: int, _frame: object) -> None:
        raise SystemExit("managed restart fixture interrupted")

    signal.signal(signal.SIGTERM, terminate)
    try:
        owner.command("bootstrap", owner.domain, str(plist))
        yield owner
    finally:
        try:
            for pid in list(owner.processes):
                owner.remember(pid)
            owner.command("bootout", f"{owner.domain}/{owner.label}", check=False)
            deadline = time.monotonic() + 15
            while owner.command("list", owner.label, check=False).returncode == 0:
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
