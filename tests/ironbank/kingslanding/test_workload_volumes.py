"""An image's VOLUME is state on the VM's overlay, owned by the image's user.

The Redis fixture declares `VOLUME /data`, owned by its `redis` user (999).
The launcher seeds the volume from the image on the VM's ext4 system
overlay, so it is owned by 999 mapped into the workload's range (100999),
Redis can save into it as itself, and a named VM keeps it across a restart.
It used to be a 64 MiB tmpfs that every restart emptied.
"""

import socket
import subprocess

from helpers.constants import BIN_DIR

from tests.ironbank.kingslanding.test_lifecycle import ping, redis
from tests.ironbank.kingslanding.test_run import environment, exec_output_text, service, wait_for

__all__ = ["redis", "service"]

REDIS_UID_MAPPED = 100000 + 999
VOLUME = "/var/lib/capsem/volumes/data-*"


def redis_command(port, command):
    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
        connection.sendall(command.encode() + b"\r\n")
        return connection.recv(64)


def vm_exec(client, vm_id, command):
    result = client.post(f"/vms/{vm_id}/exec", {"target": "vm", "command": command, "timeout_secs": 10})
    assert result.get("exit_code") == 0, result
    return exec_output_text(result)


def test_a_volume_belongs_to_the_image_user_and_survives_a_restart(redis, service):
    client, vm, port = service.client(), redis["vm"], redis["port"]
    assert redis_command(port, "SAVE") == b"+OK\r\n", "Redis saves into /data as its own user"
    owners = vm_exec(client, vm["id"], f"stat -c '%u' {VOLUME} {VOLUME}/dump.rdb").split()
    assert owners == [str(REDIS_UID_MAPPED)] * 2, owners

    result = subprocess.run(
        [str(BIN_DIR / "capsem"), "--uds-path", str(service.uds_path), "restart", vm["name"]],
        env=environment(service),
        capture_output=True,
        timeout=60,
        check=False,
    )
    assert result.returncode == 0, result.stderr

    def restarted():
        try:
            ping(port)
            return True
        except (OSError, AssertionError):
            return False

    wait_for(restarted, "Redis back after the restart", timeout=60)
    assert vm_exec(client, vm["id"], f"ls {VOLUME}").split() == ["dump.rdb"]
