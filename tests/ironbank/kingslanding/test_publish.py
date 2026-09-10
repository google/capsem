"""Redis through the real confined host TCP router and guest VSOCK path.

Kingslanding uses a pinned native image prepared before hermetic execution.
"""

import concurrent.futures
import contextlib
import re
import signal
import socket
import subprocess

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import (
    command,
    environment,
    service,
    wait_for,
)

__all__ = ["service"]
pytestmark = pytest.mark.integration


@pytest.fixture
def redis(service, tmp_path):
    with (
        registry(tmp_path) as (reference, certificate, _),
        (tmp_path / "stdout").open("wb") as stdout,
        (tmp_path / "stderr").open("wb") as stderr,
    ):
        process = subprocess.Popen(
            command(service, reference, certificate, "-p", "0:6379", "-p", "0:9099"),
            env=environment(service),
            stdout=stdout,
            stderr=stderr,
        )
        ports = []
        vm_id = None
        try:

            def ready():
                assert process.poll() is None, (tmp_path / "stderr").read_text() + (
                    tmp_path / "stdout"
                ).read_text()
                return (
                    b"Ready to accept connections tcp"
                    in (tmp_path / "stdout").read_bytes()
                )

            wait_for(ready, "Redis container startup")
            mappings = re.findall(
                r"Published 127.0.0.1:(\d+) -> (\d+)/tcp \(router (\d+)\)",
                (tmp_path / "stderr").read_text(),
            )
            assert len(mappings) == 2
            ports = [int(row[0]) for row in mappings]
            rows = service.client().get("/vms/list")["sandboxes"]
            assert len(rows) == 1
            vm_id = rows[0]["id"]
            yield {
                "port": ports[0],
                "other_port": ports[1],
                "process": process,
                "vm": rows[0],
                "reference": reference,
                "router_pids": [int(row[2]) for row in mappings],
            }
        finally:
            for log in service.tmp_dir.glob("persistent/*/process.log"):
                (tmp_path / "process.log").write_bytes(log.read_bytes())
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            if vm_id and any(
                row["id"] == vm_id
                for row in service.client().get("/vms/list")["sandboxes"]
            ):
                service.client().delete(f"/vms/{vm_id}/delete")
            assert service.client().get("/vms/list")["sandboxes"] == []
            for port in ports:

                def released(port=port):
                    with socket.socket() as listener:
                        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                        try:
                            listener.bind(("127.0.0.1", port))
                            return True
                        except OSError:
                            return False

                wait_for(released, "published listener removed", timeout=3)


def test_redis_host_tcp_concurrent_roundtrips(redis):
    def client(index):
        with (
            socket.create_connection(
                ("127.0.0.1", redis["port"]), timeout=5
            ) as connection,
            connection.makefile("rb") as stream,
        ):
            connection.sendall(
                f"PING\r\nSET key-{index} value-{index}\r\nGET key-{index}\r\n".encode()
            )
            assert stream.readline() == b"+PONG\r\n"
            assert stream.readline() == b"+OK\r\n"
            value = f"value-{index}".encode()
            assert stream.readline() == f"${len(value)}\r\n".encode()
            assert stream.readline() == value + b"\r\n"

    with concurrent.futures.ThreadPoolExecutor(max_workers=32) as executor:
        list(executor.map(client, range(64)))


def test_binary_values_and_guest_namespace_isolation(redis, service):
    value = bytes(range(256)) * 16
    with (
        socket.create_connection(("127.0.0.1", redis["port"]), timeout=5) as connection,
        connection.makefile("rb") as stream,
    ):
        connection.sendall(
            b"*3\r\n$3\r\nSET\r\n$6\r\nbinary\r\n$4096\r\n"
            + value
            + b"\r\nGET binary\r\n"
        )
        assert stream.readline() == b"+OK\r\n"
        assert stream.readline() == b"$4096\r\n"
        assert stream.read(4098) == value + b"\r\n"
    response = service.client().post(
        f"/vms/{redis['vm']['id']}/exec",
        {
            "command": 'python3 -c \'import subprocess; subprocess.Popen(["python3", "-m", "http.server", "9099", "--bind", "127.0.0.1"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)\'',
            "timeout_secs": 5,
        },
    )
    assert response.get("exit_code") == 0, response

    def guest_ready():
        response = service.client().post(
            f"/vms/{redis['vm']['id']}/exec",
            {
                "command": "python3 -c 'import urllib.request; assert urllib.request.urlopen(\"http://127.0.0.1:9099\", timeout=1).status == 200'",
                "timeout_secs": 5,
            },
        )
        return response.get("exit_code") == 0

    wait_for(guest_ready, "guest root namespace HTTP listener", timeout=5)
    with socket.create_connection(
        ("127.0.0.1", redis["other_port"]), timeout=5
    ) as connection:
        connection.sendall(b"GET / HTTP/1.0\r\n\r\n")
        with contextlib.suppress(ConnectionResetError):
            assert connection.recv(1) == b""


def test_port_collision_does_not_replace_a_listener(service, tmp_path):
    with registry(tmp_path) as (reference, certificate, _), socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]
        result = subprocess.run(
            command(service, reference, certificate, "-p", f"{port}:6379"),
            env=environment(service),
            capture_output=True,
            timeout=30,
            check=False,
        )
        assert (
            result.returncode != 0 and b"bind publication listener" in result.stderr
        ), result.stderr
        rows = service.client().get("/vms/list")["sandboxes"]
        assert len(rows) == 1 and rows[0]["status"] == "Stopped"
        service.client().delete(f"/vms/{rows[0]['id']}/delete")
        with socket.create_connection(("127.0.0.1", port), timeout=2):
            accepted, _ = listener.accept()
            accepted.close()


def test_router_crash_cannot_stop_or_control_the_vm(redis, service):
    import os

    os.kill(redis["router_pids"][0], signal.SIGKILL)
    response = service.client().post(
        f"/vms/{redis['vm']['id']}/exec",
        {"command": "printf owner-alive", "timeout_secs": 5},
    )
    assert response["exit_code"] == 0 and response["stdout"] == "owner-alive"
    assert redis["process"].poll() is None


def test_shutdown_under_load(redis, service):
    clients = [
        socket.create_connection(("127.0.0.1", redis["port"]), timeout=5)
        for _ in range(32)
    ]
    try:
        for connection in clients:
            connection.sendall(b"PING\r\n" * 1024)
        service.client().post(f"/vms/{redis['vm']['id']}/stop", {})
        redis["process"].wait(timeout=30)
        for connection in clients:
            try:
                while connection.recv(8192):
                    pass
            except ConnectionResetError:
                pass
    finally:
        for connection in clients:
            connection.close()


def test_vm_owner_death_removes_router_and_listener(redis):
    import os

    os.kill(redis["vm"]["pid"], signal.SIGKILL)
    assert redis["process"].wait(timeout=15) != 0

    # Fixture teardown verifies both listeners can be rebound and the VM is gone.
    def routers_gone():
        for pid in redis["router_pids"]:
            try:
                os.kill(pid, 0)
                return False
            except ProcessLookupError:
                pass
        return True

    wait_for(routers_gone, "router exits when VM owner dies", timeout=5)
