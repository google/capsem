"""One-shot commands use exact saved policy and clean up their temporary VMs."""

import json
import sqlite3
import subprocess
import uuid
from contextlib import closing
from http.client import HTTPConnection
from urllib.parse import urlsplit

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import TcpHttpClient
from helpers.service import exec_output_text
from helpers.service_gateway import ServiceGateway

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import cli, environment, service, wait_for

__all__ = ["service"]
pytestmark = pytest.mark.integration


def one_shot_pin_count(service):
    db = (service.home_dir / "sessions/host.db").as_uri() + "?mode=ro"
    with closing(sqlite3.connect(db, uri=True)) as connection:
        return connection.execute(
            "SELECT COUNT(*) FROM environment_revision_references WHERE owner_kind LIKE 'sandbox_%'"
        ).fetchone()[0]


def test_one_shot_command_uses_historical_pinned_variables_and_policy(service):
    saved = save(service, {
        "name": "one-shot",
        "variables": {"RUN_FIXTURE_VALUE": "original"},
        "access": {"custom": {"fixture_block": {
            "name": "Stop the disposable fixture command",
            "match": 'process.command.contains("CAPSEM_RUN_BLOCK_FIXTURE")',
            "action": "block",
        }}},
    })
    route = f"/environments/{saved['environment_id']}/changes"
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
            code, edited = operator.call_json("POST", route, {
                "request_id": str(uuid.uuid4()),
                "base_revision_id": saved["revision_id"],
                "specification": {"name": "one-shot", "variables": {"RUN_FIXTURE_VALUE": "new"}},
                "reason": "verify the command still uses the selected older revision",
            })
            assert code == 200 and edited["application"] == "applied", edited
            head = edited["result"]
            original_inventory = operator.get("/vms/list")["sandboxes"]
            options = {"environment": saved, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS}
            code, result = operator.call_json("POST", "/run", {
                **options, "command": 'printf "%s" "$RUN_FIXTURE_VALUE"',
            })
            assert code == 200 and result["exit_code"] == 0, result
            assert exec_output_text(result) == "original"
            code, blocked = operator.call_json("POST", "/run", {
                **options, "command": "printf CAPSEM_RUN_BLOCK_FIXTURE",
            })
            assert code == 200 and blocked["exit_code"] != 0, blocked
            assert exec_output_text(blocked) == ""
            code, allowed = operator.call_json("POST", "/run", {
                **options, "environment": head, "command": "printf CAPSEM_RUN_BLOCK_FIXTURE",
            })
            assert code == 200 and allowed["exit_code"] == 0, allowed
            assert exec_output_text(allowed) == "CAPSEM_RUN_BLOCK_FIXTURE"
            assert operator.call_json("POST", "/run", {
                **options, "command": "true", "env": {"RUN_FIXTURE_VALUE": "inline"},
            })[0] == 400
            status, timed_out = operator.call_json("POST", "/run", {
                **options, "command": "sleep 3", "timeout_secs": 1,
            })
            assert status == 504, timed_out
            assert operator.get("/vms/list")["sandboxes"] == original_inventory
            assert one_shot_pin_count(service) == 0

            selected = f"{saved['environment_id']}@{saved['revision_id']}"
            exited = subprocess.run(
                cli(service, "run", "--ram", str(DEFAULT_RAM_MB // 1024), "--cpu", str(DEFAULT_CPUS),
                    "--environment", selected, 'printf "%s" "$RUN_FIXTURE_VALUE"'),
                env=environment(service), capture_output=True, timeout=90, check=False,
            )
            assert exited.returncode == 0, exited.stderr.decode(errors="replace")
            assert exited.stdout == b"original"
            assert operator.get("/vms/list")["sandboxes"] == original_inventory
            assert one_shot_pin_count(service) == 0
            code, _ = operator.call_json("POST", route, {
                "action": "archive", "request_id": str(uuid.uuid4()),
                "base_revision_id": head["revision_id"], "reason": "retire the disposable run fixture",
            })
            assert code == 200
            assert operator.call_json("POST", "/run", {**options, "command": "true"})[0] == 409
            assert operator.get("/vms/list")["sandboxes"] == original_inventory
    finally:
        gateway.stop()


def test_run_rejects_a_workload_revision_before_provisioning(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        saved = save(service, {
            "name": "one-shot-workload",
            "image": {"reference": "redis"},
            "startup": {"script": "printf 'workload fixture'", "timeout_seconds": 10},
            "mcps": {"servers": {"fixture": {"kind": "sandbox", "executable": "echo"}}},
        })
        before = service.client().get("/vms/list")["sandboxes"]
        status, result = service.client().call_json("POST", "/run", {
            "environment": saved, "command": "true", "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        assert status == 400 and "one-shot command" in result["error"], result
        selected = f"{saved['environment_id']}@{saved['revision_id']}"
        refused = subprocess.run(
            cli(service, "run", "--environment", selected, "true"),
            env=environment(service), capture_output=True, timeout=30, check=False,
        )
        assert refused.returncode != 0 and b"one-shot command" in refused.stderr, refused.stderr
        assert service.client().get("/vms/list")["sandboxes"] == before
        assert one_shot_pin_count(service) == 0


def test_one_shot_retirement_finishes_after_gateway_client_disconnects(service):
    saved = save(service, {"name": "one-shot-disconnect"})
    gateway = ServiceGateway(service)
    gateway.start()
    try:
        authority = urlsplit(gateway.base_url)
        with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator, closing(
            HTTPConnection(authority.hostname, authority.port, timeout=10)
        ) as impatient:
            assert operator.get("/vms/list")["sandboxes"] == []
            impatient.request("POST", "/run", body=json.dumps({
                "environment": saved, "command": "sleep 2", "timeout_secs": 10,
                "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            }), headers={"Authorization": f"Bearer {gateway.token}", "Content-Type": "application/json"})
            wait_for(lambda: one_shot_pin_count(service) == 3, "running one-shot pins", timeout=10)
            impatient.close()
            wait_for(lambda: operator.get("/vms/list")["sandboxes"] == [], "one-shot VM retirement", timeout=15)
            wait_for(lambda: one_shot_pin_count(service) == 0, "one-shot pin release", timeout=10)
    finally:
        gateway.stop()
