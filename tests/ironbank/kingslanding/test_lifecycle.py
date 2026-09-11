"""Existing VM lifecycle owns containers; closing the CLI only detaches.

Uses the native Redis fixture prepared by the Kingslanding gate.
"""

import signal
import socket
import subprocess

import pytest
from helpers.constants import BIN_DIR

from tests.ironbank.kingslanding.test_publish import redis, service
from tests.ironbank.kingslanding.test_run import environment, wait_for

__all__ = ["redis", "service"]


def ping(port):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
        connection.sendall(b"PING\r\n")
        assert connection.recv(7) == b"+PONG\r\n"


@pytest.mark.parametrize("signal_number", [signal.SIGINT, signal.SIGKILL])
def test_cli_detaches_and_existing_restart_restores_redis(
    redis, service, signal_number
):
    process = redis["process"]
    vm = redis["vm"]
    client = service.client()
    try:
        process.send_signal(signal_number)
        process.wait(timeout=5)
        ping(redis["port"])
        assert client.get(f"/vms/{vm['id']}/info")["persistent"]
        result = subprocess.run(
            [
                str(BIN_DIR / "capsem"),
                "--uds-path",
                str(service.uds_path),
                "restart",
                vm["name"],
            ],
            env=environment(service),
            capture_output=True,
            timeout=60,
            check=False,
        )
        assert result.returncode == 0, result.stderr

        def restarted():
            try:
                ping(redis["port"])
                return True
            except (OSError, AssertionError):
                return False

        wait_for(restarted, "Redis restarted through existing VM command", timeout=30)
        client.post(f"/vms/{vm['id']}/stop", {})
        with pytest.raises(OSError):
            socket.create_connection(("127.0.0.1", redis["port"]), timeout=1)
        assert client.get(f"/vms/{vm['id']}/info")["status"] == "Stopped"
    finally:
        client.delete(f"/vms/{vm['id']}/delete")


def test_existing_fork_starts_saved_container_without_stealing_ports(redis, service):
    client = service.client()
    source = redis["vm"]["id"]
    marker = client.post(
        f"/vms/{source}/exec",
        {"command": "printf fork-proof > /root/fork-marker", "timeout_secs": 5},
    )
    assert marker["exit_code"] == 0
    fork = client.post(
        f"/vms/{source}/fork",
        {"name": "redis-fork", "description": "container lifecycle proof"},
    )
    fork_id = fork["id"]
    try:
        client.post(f"/vms/{fork_id}/resume", {})

        def fork_ready():
            result = client.post(
                f"/vms/{fork_id}/exec",
                {
                    "command": "set -eu; test $(cat /root/fork-marker) = fork-proof; pid=$(cat /var/tmp/capsem-container/workload.pid); nsenter -t $pid -n /bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/6379; printf \"PING\\r\\n\" >&3; head -c 7 <&3'",
                    "timeout_secs": 5,
                },
            )
            return result.get("exit_code") == 0 and result.get("stdout") == "+PONG\r\n"

        wait_for(fork_ready, "fork boots saved image and command", timeout=30)
        ping(redis["port"])
        assert client.get(f"/vms/{source}/info")["pid"] == redis["vm"]["pid"]
    finally:
        client.delete(f"/vms/{fork_id}/delete")
