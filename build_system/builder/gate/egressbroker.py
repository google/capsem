"""Unsandboxed process half of :mod:`capsem_builder.gate.egress`.

Kept separate so the resource/capability protocol stays below the gate module
ceiling.  This process owns no orchestration state; it accepts one authenticated
command at a time and exits when its pre-sandbox parent or capability goes.
"""

from __future__ import annotations

import hmac
import os
import socket
import subprocess
import sys
from pathlib import Path

from .egress import _receive, _send


def _execute(request: dict) -> dict:
    argv = [str(part) for part in request["argv"]]
    environment = {**os.environ, **{str(k): str(v) for k, v in request["env"].items()}}
    cwd = str(request["cwd"])
    if request["capture"]:
        completed = subprocess.run(
            argv, cwd=cwd, env=environment, text=True, capture_output=True, check=False
        )
        return {
            "ok": True,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }

    log = Path(request["log"]) if request.get("log") else None
    if log is None:
        completed = subprocess.run(argv, cwd=cwd, env=environment, check=False)
        return {"ok": True, "returncode": completed.returncode}
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("a", encoding="utf-8", buffering=1) as target:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=environment,
            text=True,
            bufsize=1,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        assert process.stdout is not None
        for line in process.stdout:
            target.write(line)
            sys.stderr.write(line)
            sys.stderr.flush()
        process.wait()
    return {"ok": True, "returncode": process.returncode}


def serve(endpoint: Path, parent: int, maximum: int) -> int:
    token = sys.stdin.readline().strip()
    if not token:
        return 2
    endpoint.parent.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(endpoint))
        os.chmod(endpoint, 0o600)
        listener.listen(1)
        listener.settimeout(0.25)
        while os.getppid() == parent:
            try:
                connection, _ = listener.accept()
            except TimeoutError:
                continue
            with connection:
                try:
                    request = _receive(connection, maximum)
                    if not hmac.compare_digest(str(request.pop("token", "")), token):
                        response = {"ok": False, "error": "release egress authentication failed"}
                    elif request.get("op") == "shutdown":
                        _send(connection, {"ok": True}, maximum)
                        return 0
                    elif request.get("op") == "execute":
                        response = _execute(request)
                    else:
                        response = {"ok": False, "error": "unknown release egress operation"}
                except Exception as error:  # preserve the boundary failure, not a raw EOF
                    response = {"ok": False, "error": f"release egress failed: {error}"}
                _send(connection, response, maximum)
    return 0


if __name__ == "__main__":
    raise SystemExit(serve(Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])))
