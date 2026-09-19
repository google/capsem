"""Real expose authorization: deny before Redis accepts a TCP connection."""

import contextlib
import json
import re
import socket
import sqlite3

import pytest
from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_publish import redis
from tests.ironbank.kingslanding.test_run import service, wait_for

__all__ = ["redis", "service"]
pytestmark = pytest.mark.integration


def test_container_pull_policy_stops_before_registry_egress_and_redacts_credentials(service, tmp_path):
    client = service.client()
    name = "container-pull-denied"
    password = "registry-password-must-not-leak"
    username = "registry-user-must-not-leak"

    with registry(tmp_path) as (reference, certificate, requests):
        registry_host = reference.split("/", 1)[0]
        result = client.put(
            f"/profiles/{CODE_PROFILE_ID}/enforcement/rules/container_pull_test/edit",
            {
                "name": "container_pull_test",
                "action": "block",
                "match": (
                    f'container.registry == "{registry_host}" && '
                    f'container.image == "{reference}"'
                ),
                "reason": "Kingslanding container pull boundary proof.",
            },
        )
        assert result["rule"]["action"] == "block"

        created = client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
                "container": {
                    "image": reference,
                    "env": {},
                    "registry": {
                        "username": username,
                        "password": password,
                        "ca_pem": certificate.read_text(),
                    },
                },
            },
            timeout=90,
        )
        # Create waits for the workload, so the refusal is its answer. The VM
        # is discarded and its name freed, but its ledger and logs are kept as
        # a failed session: they are the record of the refusal.
        assert "id" not in created, created
        refusal = created["error"]
        assert "policy refused" in refusal, created
        assert not requests, requests
        assert password not in refusal and username not in refusal
        vm_id = re.search(r"for VM ([0-9a-f-]{36})", refusal).group(1)
        assert all(vm["id"] != vm_id for vm in client.get("/vms/list")["sandboxes"])

        kept = sorted((service.tmp_dir / "sessions").glob(f"{vm_id}-failed-*"))
        assert len(kept) == 1, f"the refused create's ledger is kept: {kept}"
        with contextlib.closing(sqlite3.connect(f"file:{kept[0] / 'session.db'}?mode=ro", uri=True)) as db:
            rows = [
                {"event_type": event_type, "event_json": event_json}
                for event_type, event_json in db.execute("SELECT event_type, event_json FROM security_rule_events")
            ]
        assert any(
            row["event_type"] == "network.lifecycle"
            and json.loads(row["event_json"]).get("container", {}).get("image") == reference
            for row in rows
        ), rows
        rendered = json.dumps(rows)
        assert reference in rendered and registry_host in rendered
        assert password not in rendered and username not in rendered
        logs = "\n".join(
            path.read_text(errors="replace")
            for root in (service.home_dir, service.tmp_dir)
            for path in root.rglob("*.log*")
            if path.is_file()
        )
        assert password not in logs and username not in logs


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
    # Bound first, so the audited peer is known even when the refusal's reset
    # reaches connect() itself -- which it does on a loaded host.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as connection:
        connection.settimeout(2)
        connection.bind(("127.0.0.1", 0))
        peer = connection.getsockname()[1]
        try:
            connection.connect(("127.0.0.1", port))
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
            # An unnamed VM is known by its route id; its list label is the UI's.
            assert facts["destination"]["vm"]["name"] == redis["vm"]["id"]
            assert int(facts["destination"]["vm"]["generation"]) > 0
            assert facts["route"]["listener"] == f"127.0.0.1:{port}"
            assert facts["route"]["publication_id"] and facts["connection_id"]
            assert facts["protocol"] == "tcp" and facts["side"] == "destination"
