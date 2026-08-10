"""The narrow unsandboxed capability used by networked release edges.

Seatbelt and a Linux network namespace are inherited across ``exec`` and
cannot be removed.  A release nevertheless has to resolve its serialized
manifest before qualification and publish afterwards.  The plan executor
therefore stays sandboxed for its entire life while a helper created just
before that boundary executes only actions explicitly marked as outside it.

The helper is not a gate: it owns no plan, lock, workspace, source identity or
journal.  The one plan executor retains all of those and records every brokered
command through the same ``GuardedRunner`` as an ordinary invocation.  Its
capability token is handed over in a mode-0600 one-time file, read and deleted
before the first plan action; subprocesses spawned by qualification inherit
neither the token nor a file from which to recover it.
"""

from __future__ import annotations

import atexit
import contextlib
import json
import os
import secrets
import socket
import stat
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from .errors import GateError
from .funnel import GuardedRunner
from .invocation import Command
from .lifecycle import Resource
from .proc import Completed, Runner


def _send(connection: socket.socket, payload: dict, maximum: int) -> None:
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    if len(encoded) > maximum:
        raise GateError(f"release egress message exceeds its {maximum}-byte bound")
    connection.sendall(len(encoded).to_bytes(8, "big") + encoded)


def _receive(connection: socket.socket, maximum: int) -> dict:
    header = _read_exact(connection, 8)
    length = int.from_bytes(header, "big")
    if length <= 0 or length > maximum:
        raise GateError(f"invalid release egress message length {length}")
    value = json.loads(_read_exact(connection, length))
    if not isinstance(value, dict):
        raise GateError("release egress message is not an object")
    return value


def _read_exact(connection: socket.socket, length: int) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    while remaining:
        chunk = connection.recv(remaining)
        if not chunk:
            raise GateError("release egress connection closed mid-message")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


class EgressRunner(Runner):
    """A ``Runner`` whose actual process is the pre-sandbox helper."""

    def __init__(self, root: Path, *, endpoint: Path, token: str, maximum: int) -> None:
        super().__init__(root)
        self._endpoint = endpoint
        self._token = token
        self._maximum = maximum

    def _request(self, payload: dict) -> dict:
        payload["token"] = self._token
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.connect(str(self._endpoint))
                _send(connection, payload, self._maximum)
                response = _receive(connection, self._maximum)
        except OSError as error:
            raise GateError(f"release egress broker is unavailable: {error}") from error
        if response.get("ok") is not True:
            raise GateError(str(response.get("error") or "release egress broker refused command"))
        return response

    def execute(self, command: Command) -> Completed:
        response = self._request(
            {
                "op": "execute",
                "argv": list(command.argv),
                "cwd": str(command.cwd or self.root),
                "env": command.env,
                "capture": command.capture,
                "log": str(command.log) if command.log is not None else None,
            }
        )
        return subprocess.CompletedProcess(
            args=list(command.argv),
            returncode=int(response["returncode"]),
            stdout=response.get("stdout"),
            stderr=response.get("stderr"),
        )

    def shutdown(self) -> None:
        self._request({"op": "shutdown"})


class Egress(Resource, name="release-egress"):
    """Acquire the one-time capability prepared outside the sandbox."""

    def __init__(self, config, *, enabled: bool) -> None:
        self._config = config
        self._enabled = enabled
        self._runner: EgressRunner | None = None
        self._endpoint: Path | None = None

    @property
    def runner(self) -> Runner:
        if self._runner is None:
            raise GateError("release egress capability was not acquired")
        return self._runner

    def acquire(self) -> None:
        if not self._enabled:
            return
        variable = self._config.sandbox.egress_metadata_variable
        raw = os.environ.get(variable, "").strip()
        if not raw:
            raise GateError(
                "sandboxed release has no outside-egress capability; start it through "
                "the checked-in release command rather than from an existing sandbox"
            )
        metadata = Path(raw)
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
        try:
            descriptor = os.open(metadata, flags)
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise GateError("release egress metadata is not an owner-controlled file")
            if stat.S_IMODE(info.st_mode) != 0o600:
                raise GateError("release egress metadata must have mode 0600")
            with os.fdopen(descriptor, encoding="utf-8") as source:
                payload = json.load(source)
        except OSError as error:
            raise GateError(f"cannot read release egress metadata: {error}") from error
        finally:
            metadata.unlink(missing_ok=True)
            os.environ.pop(variable, None)

        self._endpoint = Path(payload["socket"])
        self._runner = EgressRunner(
            self._config.root,
            endpoint=self._endpoint,
            token=str(payload["token"]),
            maximum=self._config.sandbox.egress_max_message_bytes,
        )

    def release(self) -> None:
        if self._runner is not None:
            with contextlib.suppress(Exception):
                self._runner.shutdown()
        if self._endpoint is not None:
            self._endpoint.unlink(missing_ok=True)
        self._runner = None


@dataclass
class _Prepared:
    process: subprocess.Popen
    endpoint: Path
    metadata: Path
    token: str
    maximum: int
    stop_timeout: float

    def cleanup(self) -> None:
        if self.process.poll() is None:
            with contextlib.suppress(Exception):
                EgressRunner(
                    Path.cwd(),
                    endpoint=self.endpoint,
                    token=self.token,
                    maximum=self.maximum,
                ).shutdown()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.process.wait(timeout=self.stop_timeout)
        if self.process.poll() is None:
            self.process.kill()
            with contextlib.suppress(subprocess.TimeoutExpired):
                self.process.wait(timeout=self.stop_timeout)
        self.endpoint.unlink(missing_ok=True)
        self.metadata.unlink(missing_ok=True)


_PREPARED: list[_Prepared] = []


def prepare(config, directory: Path) -> Path:
    """Start the helper now, returning its one-time capability metadata."""
    settings = config.sandbox
    directory.mkdir(parents=True, exist_ok=True)
    nonce = secrets.token_hex(8)
    token = secrets.token_urlsafe(32)
    fields = {"pid": os.getpid(), "nonce": nonce}
    endpoint = Path(settings.egress_socket_template.format(**fields))
    metadata = directory / settings.egress_metadata_template.format(**fields)
    endpoint.unlink(missing_ok=True)
    process = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "capsem.gate.egressbroker",
            str(endpoint),
            str(os.getpid()),
            str(settings.egress_max_message_bytes),
        ],
        stdin=subprocess.PIPE,
        start_new_session=True,
        text=True,
    )
    assert process.stdin is not None
    process.stdin.write(token + "\n")
    process.stdin.close()

    deadline = time.monotonic() + settings.egress_start_timeout
    while not endpoint.is_socket() and process.poll() is None and time.monotonic() < deadline:
        time.sleep(0.02)
    if not endpoint.is_socket():
        process.kill()
        raise GateError("release egress broker did not become ready")

    descriptor = os.open(metadata, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as target:
        json.dump({"socket": str(endpoint), "token": token}, target)
    prepared = _Prepared(
        process, endpoint, metadata, token, settings.egress_max_message_bytes,
        settings.egress_stop_timeout,
    )
    _PREPARED.append(prepared)
    atexit.register(prepared.cleanup)
    return metadata


def runner_of(resources: tuple[Resource, ...]) -> Runner | None:
    """The acquired capability runner, when this command requested one."""
    for resource in resources:
        if isinstance(resource, Egress) and resource._enabled:
            return resource.runner
    return None


def guarded_runner_of(
    resources: tuple[Resource, ...], *, journal, tail_lines: int, checkpoint
) -> Runner | None:
    """Capability runner with the owning plan's guards and journal attached."""
    runner = runner_of(resources)
    if runner is None:
        return None
    return GuardedRunner(
        runner,
        journal=journal,
        tail_lines=tail_lines,
        checkpoint=checkpoint,
    )
