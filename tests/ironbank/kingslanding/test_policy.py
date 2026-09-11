"""Real expose authorization: deny before Redis accepts a TCP connection."""

import json
import socket

import pytest
from helpers.constants import CODE_PROFILE_ID

from tests.ironbank.kingslanding.test_publish import redis
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["redis", "service"]
pytestmark = pytest.mark.integration


def _redis_command(stream, *arguments):
    stream.write(f"*{len(arguments)}\r\n".encode())
    for argument in arguments:
        data = argument.encode()
        stream.write(f"${len(data)}\r\n".encode() + data + b"\r\n")
    stream.flush()
    line = stream.readline()
    if line.startswith(b"+"):
        return line[1:-2]
    assert line.startswith(b"$"), line
    length = int(line[1:-2])
    assert 0 <= length < 65536
    value = stream.read(length)
    assert len(value) == length and stream.read(2) == b"\r\n"
    return value


def _accepted_count(stream):
    stats = _redis_command(stream, "INFO", "stats").decode()
    return int(
        next(
            line.split(":")[1]
            for line in stats.splitlines()
            if line.startswith("total_connections_received:")
        )
    )


def _probe(port):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
        peer = connection.getsockname()[1]
        try:
            connection.sendall(b"*1\r\n$4\r\nPING\r\n")
            with connection.makefile("rb") as response:
                reply = response.readline(32)
        except (ConnectionResetError, BrokenPipeError):
            return True, peer
        assert reply in (b"", b"+PONG\r\n"), reply
        return not reply, peer


@pytest.mark.parametrize("policy", ["block", "ask", "plugin"])
def test_expose_security_prevents_redis_accept_and_retains_trusted_facts(redis, service, policy):
    client = service.client()
    vm_id = redis["vm"]["id"]
    port = redis["port"]
    with (
        socket.create_connection(("127.0.0.1", port), timeout=3) as observer,
        observer.makefile("rwb") as stream,
    ):
        assert _redis_command(stream, "PING") == b"PONG"
        if policy == "plugin":
            result = client.patch(
                f"/profiles/{CODE_PROFILE_ID}/plugins/dummy_post_allow/edit",
                {"mode": "block", "detection_level": "high"},
            )
            assert result["config"]["mode"] == "block"
        else:
            result = client.put(
                f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/expose_test/edit",
                {
                    "name": "expose_test",
                    "action": policy,
                    "match": 'network.mode == "expose" && network.destination.port == "6379"',
                    "reason": "Kingslanding expose boundary proof.",
                },
            )
            assert result["rule"]["action"] == policy

        # The edit route returns only after the running owner acknowledged the
        # reload, so the very next connection must already be refused. Measure
        # through the same observer socket so measurement does not perturb
        # Redis's count.
        assert _probe(port)[0], "edited policy still admitted a connection"
        baseline = _accepted_count(stream)
        denied_peers = set()
        for _ in range(10):
            denied, peer = _probe(port)
            assert denied
            denied_peers.add(f"127.0.0.1:{peer}")
        assert _accepted_count(stream) == baseline, "denial still connected to Redis"
        assert _redis_command(stream, "PING") == b"PONG"

        rows = []

        def audited():
            rows[:] = client.get(f"/vms/{vm_id}/security/latest?limit=2000")
            seen = {
                json.loads(row["event_json"])["network"]["source"]["address"]
                for row in rows
                if row["event_type"] == "network.connect"
            }
            return denied_peers <= seen

        wait_for(audited, "denied connection security rows", timeout=15)
        for row in rows:
            if row["event_type"] != "network.connect":
                continue
            event = json.loads(row["event_json"])
            facts = event["network"]
            if facts["source"]["address"] not in denied_peers:
                continue
            assert event["decision"]["effective"] == ("ask" if policy == "ask" else "block")
            assert facts["source"]["vm"] is None
            assert facts["destination"]["address"] == "127.0.0.1:6379"
            assert facts["destination"]["vm"]["id"] == vm_id
            assert facts["destination"]["vm"]["name"] == redis["vm"]["name"]
            assert int(facts["destination"]["vm"]["generation"]) > 0
            assert facts["route"]["listener"] == f"127.0.0.1:{port}"
            assert facts["route"]["publication_id"] and facts["connection_id"]
            assert facts["protocol"] == "tcp" and facts["side"] == "destination"
