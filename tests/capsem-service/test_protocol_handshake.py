"""A silent unauthorized IPC client is refused without poisoning its VM."""

import json
import socket
from pathlib import Path

import psutil
import pytest
from helpers.service import exec_output_text, vm_session_dir, wait_exec_ready
from log_streams import read_log_stream

pytestmark = pytest.mark.integration


def _per_vm_socket(service_env, vm_id: str) -> Path:
    """Observe the service-owned child's selected socket, including short paths."""
    assert service_env.proc is not None
    commands = [child.cmdline() for child in psutil.Process(service_env.proc.pid).children()]
    owners = [
        args for args in commands
        if args and Path(args[0]).name == "capsem-process"
        and "--id" in args and args[args.index("--id") + 1] == vm_id
    ]
    assert len(owners) == 1, owners
    args = owners[0]
    return Path(args[args.index("--uds-path") + 1])


def test_pre_handshake_client_disconnects_quickly(service_env, fresh_vm):
    """Only the owning service may negotiate Hello or dispatch VM commands."""
    _, created = fresh_vm()
    assert isinstance(created, dict) and created.get("id"), created
    vm_id = created["id"]
    client = service_env.client()
    assert wait_exec_ready(client, vm_id), "VM never became exec-ready"
    sock_path = _per_vm_socket(service_env, vm_id)
    assert sock_path.is_socket(), sock_path
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
        peer.settimeout(2)
        peer.connect(str(sock_path))
        # Keep the peer open and send no Hello: the owner must close it first.
        assert peer.recv(1) == b"", "unauthorized peer received IPC bytes"

    result = client.post(f"/vms/{vm_id}/exec", {
        "target": "vm", "command": "printf IPC_STILL_READY", "timeout_secs": 10,
    })
    assert result and result["exit_code"] == 0, result
    assert exec_output_text(result) == "IPC_STILL_READY"
    session = vm_session_dir(service_env.tmp_dir, client, vm_id)
    records = [json.loads(line) for line in read_log_stream(session / "process.log").splitlines()]
    assert any(
        record.get("target") == "ipc"
        and record.get("fields", {}).get("message") == "unauthorized IPC peer; refusing connection before Hello"
        for record in records
    ), "owner did not record its pre-Hello peer refusal"
