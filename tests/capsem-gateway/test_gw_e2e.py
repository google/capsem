"""Gateway end-to-end tests with real capsem-service + VMs.

These tests boot real VMs through the gateway TCP endpoint.
Requires capsem-service binary, VM assets, and codesigned binaries.
"""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT, EXEC_TIMEOUT_SECS, HTTP_TIMEOUT
from helpers.gateway import GatewayInstance, TcpHttpClient
from helpers.profile_asset_fixture import asset_source_dir, find_asset, write_profile_home
from helpers.service import ServiceInstance, vm_name

pytestmark = [pytest.mark.gateway, pytest.mark.e2e]


@pytest.fixture(scope="module")
def e2e_env(tmp_path_factory):
    """Start real capsem-service + capsem-gateway with Profile V2 assets."""
    source_dir = asset_source_dir()
    if not source_dir.exists():
        pytest.skip(f"asset source dir missing: {source_dir}")

    assets = {
        "vmlinuz": find_asset(source_dir, "vmlinuz"),
        "initrd.img": find_asset(source_dir, "initrd.img"),
        "rootfs.squashfs": find_asset(source_dir, "rootfs.squashfs"),
    }
    capsem_home = tmp_path_factory.mktemp("gw-profile-home")
    asset_cache = tmp_path_factory.getbasetemp() / f"gw-profile-assets-{uuid.uuid4().hex[:8]}"
    write_profile_home(capsem_home, asset_cache, assets)

    svc = ServiceInstance(
        extra_env={
            "CAPSEM_HOME": str(capsem_home),
            "CAPSEM_ASSETS_DIR": str(asset_cache),
        },
        assets_dir=asset_cache,
    )
    svc.start()
    gw = GatewayInstance(uds_path=svc.uds_path)
    gw.start()
    yield gw, svc
    gw.stop()
    svc.stop()


@pytest.fixture
def e2e_client(e2e_env):
    gw, _ = e2e_env
    return TcpHttpClient(gw.base_url, gw.token)


class TestGatewayE2E:

    def test_profile_selected_create_download_boot_via_gateway(self, e2e_client):
        """HTTP create selects a profile, reconciles its assets, boots, and pins."""
        status_before = e2e_client.get("/setup/assets", timeout=30)
        assert status_before["profile_id"] == "profile-asset-boot"

        name = vm_name("gw-profile")
        resp = e2e_client.post(
            "/provision",
            {
                "name": name,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "profile_id": "profile-asset-boot",
                "profile_revision": "2026.0519.e2e",
            },
            timeout=240,
        )
        assert resp is not None, "profile-selected provision failed"
        assert "error" not in resp, resp
        vm_id = resp.get("id", name)
        assert resp["profile_id"] == "profile-asset-boot"
        assert resp["profile_revision"] == "2026.0519.e2e"
        assert resp["profile_status"] == "current"
        assert resp["profile_pin"]["profile_id"] == "profile-asset-boot"
        assert resp["profile_pin"]["profile_revision"] == "2026.0519.e2e"
        assert resp["profile_pin"]["profile_payload_hash"].startswith("blake3:")
        assert resp["profile_pin"]["package_contract_hash"].startswith("blake3:")
        assert resp["profile_pin"]["base_assets"]["rootfs_hash"]
        assert resp["asset_health"]["ready"] is True
        assert resp["asset_health"]["profile_id"] == "profile-asset-boot"
        assert len(resp["asset_health"]["profile_assets"]) == 3

        try:
            assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=180), (
                f"VM {vm_id} never became exec-ready through gateway"
            )
            exec_resp = e2e_client.post(
                f"/exec/{vm_id}",
                {"command": "echo gateway-profile-boot"},
                timeout=HTTP_TIMEOUT,
            )
            assert "gateway-profile-boot" in exec_resp.get("stdout", "")

            info = e2e_client.get(f"/info/{vm_id}", timeout=60)
            assert info["profile_id"] == "profile-asset-boot"
            assert info["profile_revision"] == "2026.0519.e2e"
            assert info["profile_status"] == "current"
        finally:
            e2e_client.delete(f"/delete/{vm_id}")

    def test_provision_list_exec_stop_delete(self, e2e_client):
        """Full VM lifecycle through gateway TCP endpoint."""
        name = vm_name("gw-e2e")
        # Provision
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        assert resp is not None, "provision failed"
        vm_id = resp.get("id", name)

        # Wait for exec-ready
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60), (
            f"VM {vm_id} never became exec-ready through gateway"
        )

        # List -- VM should appear
        listing = e2e_client.get("/list")
        assert listing is not None
        ids = [s["id"] for s in listing.get("sandboxes", [])]
        assert vm_id in ids, f"VM {vm_id} not in list: {ids}"

        # Exec
        exec_resp = e2e_client.post(f"/exec/{vm_id}", {
            "command": "echo gateway-works",
        })
        assert exec_resp is not None
        assert "gateway-works" in exec_resp.get("stdout", "")
        assert exec_resp.get("exit_code") == 0

        # Stop + Delete
        e2e_client.post(f"/stop/{vm_id}", {})
        e2e_client.delete(f"/delete/{vm_id}")

        # Verify removed
        listing = e2e_client.get("/list")
        ids = [s["id"] for s in listing.get("sandboxes", [])]
        assert vm_id not in ids

    def test_status_with_running_vm(self, e2e_client):
        """GET /status shows running VMs with resource summary."""
        name = vm_name("gw-st")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            status = e2e_client.get("/status")
            assert status is not None
            assert status.get("service") == "running"
            assert status.get("vm_count", 0) >= 1
            rs = status.get("resource_summary")
            assert rs is not None
            assert rs.get("running_count", 0) >= 1
        finally:
            e2e_client.delete(f"/delete/{vm_id}")

    def test_404_for_nonexistent_vm(self, e2e_client):
        """Error for nonexistent VM is proxied correctly."""
        resp = e2e_client.get("/info/ghost-vm-does-not-exist")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()

    def test_immediate_exec_after_provision(self, e2e_client):
        """Regression: exec immediately after provision, NO polling.

        Same pattern as test_svc_exec_ready.py but exercising the full
        TCP -> gateway -> UDS -> service -> process -> VM path.
        The server must handle readiness internally through the proxy chain.
        """
        name = vm_name("gw-race")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        assert resp is not None, "provision failed"
        vm_id = resp.get("id", name)

        # Immediately exec -- NO wait_exec_ready, NO sleep.
        # Server must internally wait for VM readiness.
        try:
            exec_resp = e2e_client.post(
                f"/exec/{vm_id}",
                {"command": "echo race-ok", "timeout_secs": EXEC_TIMEOUT_SECS},
                timeout=HTTP_TIMEOUT,
            )
            assert exec_resp is not None, "exec returned None"
            assert "race-ok" in exec_resp.get("stdout", ""), (
                f"expected 'race-ok' in stdout, got: {exec_resp}"
            )
            assert exec_resp.get("exit_code") == 0
        finally:
            e2e_client.delete(f"/delete/{vm_id}")

    def test_health_while_vm_running(self, e2e_env):
        """Health endpoint works even with VMs running."""
        gw, _ = e2e_env
        import json
        import subprocess
        result = subprocess.run(
            ["curl", "-s", "--max-time", "5",
             f"http://127.0.0.1:{gw.port}/"],
            capture_output=True, text=True, timeout=10,
        )
        data = json.loads(result.stdout)
        assert data["ok"] is True


class TestGatewayFileIO:
    """File read/write operations through the gateway."""

    def test_write_and_read_file_through_gateway(self, e2e_client):
        """Write a file to guest, then read it back through gateway."""
        name = vm_name("gw-file")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            # Write file
            write_resp = e2e_client.write_file(vm_id, "/root/gw-test.txt", "gateway file io test")
            assert write_resp is not None

            # Read file back
            read_resp = e2e_client.read_file(vm_id, "/root/gw-test.txt")
            assert read_resp is not None
            assert "gateway file io test" in str(read_resp)
        finally:
            e2e_client.delete(f"/delete/{vm_id}")

    def test_write_binary_content(self, e2e_client):
        """Write a file with special characters."""
        name = vm_name("gw-bin")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            write_resp = e2e_client.write_file(vm_id, "/root/special.txt", "line1\nline2\ttab\n")
            assert write_resp is not None

            exec_resp = e2e_client.post(f"/exec/{vm_id}", {
                "command": "wc -l /root/special.txt",
            })
            assert exec_resp is not None
            # Should have 2-3 lines
            assert exec_resp.get("exit_code") == 0
        finally:
            e2e_client.delete(f"/delete/{vm_id}")


class TestGatewayPersistence:
    """Persistent VM operations through the gateway."""

    def test_persist_and_resume_through_gateway(self, e2e_client):
        """Create ephemeral VM, persist it, stop, resume through gateway."""
        name = vm_name("gw-persist")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            "persistent": True,
        })
        assert resp is not None
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            # Write a marker file
            e2e_client.write_file(vm_id, "/root/persist-marker.txt", "survived-restart")

            # Stop
            e2e_client.post(f"/stop/{vm_id}", {})
            import time
            time.sleep(2)

            # Resume
            resume_resp = e2e_client.post(f"/resume/{name}", {})
            assert resume_resp is not None

            # Wait for exec ready again
            resumed_id = resume_resp.get("id", name)
            assert wait_exec_ready_tcp(e2e_client, resumed_id, timeout=60)

            # Check marker file survived
            exec_resp = e2e_client.post(f"/exec/{resumed_id}", {
                "command": "cat /root/persist-marker.txt",
            })
            assert exec_resp is not None
            assert "survived-restart" in exec_resp.get("stdout", "")
        finally:
            e2e_client.delete(f"/delete/{vm_id}")

    def test_purge_through_gateway(self, e2e_client):
        """POST /purge kills ephemeral VMs through gateway."""
        name = vm_name("gw-purge")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        assert resp is not None

        # Purge
        purge_resp = e2e_client.post("/purge", {})
        assert purge_resp is not None

        # VM should be gone
        listing = e2e_client.get("/list")
        ids = [s["id"] for s in listing.get("sandboxes", [])]
        assert name not in ids


class TestGatewayLogs:
    """Log retrieval through the gateway."""

    def test_logs_for_running_vm(self, e2e_client):
        """GET /logs/{id} returns boot logs for a running VM."""
        name = vm_name("gw-logs")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            logs_resp = e2e_client.get(f"/logs/{vm_id}")
            assert logs_resp is not None
            assert "logs" in logs_resp
        finally:
            e2e_client.delete(f"/delete/{vm_id}")


class TestGatewayEnvVars:
    """Environment variable injection through the gateway."""

    def test_env_vars_passed_to_guest(self, e2e_client):
        """Environment variables are passed through gateway to the guest."""
        name = vm_name("gw-env")
        resp = e2e_client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            "env": {"GW_TEST_VAR": "hello-from-gateway"},
        })
        assert resp is not None
        vm_id = resp.get("id", name)
        assert wait_exec_ready_tcp(e2e_client, vm_id, timeout=60)

        try:
            exec_resp = e2e_client.post(f"/exec/{vm_id}", {
                "command": "echo $GW_TEST_VAR",
            })
            assert exec_resp is not None
            assert "hello-from-gateway" in exec_resp.get("stdout", "")
        finally:
            e2e_client.delete(f"/delete/{vm_id}")


def wait_exec_ready_tcp(client, vm_id, timeout=EXEC_READY_TIMEOUT):
    """Wait until VM responds to exec through gateway.

    The server polls internally for VM readiness, so a single call with
    adequate timeout is sufficient.
    """
    try:
        resp = client.post(
            f"/exec/{vm_id}",
            {"command": "echo ready", "timeout_secs": timeout},
            timeout=timeout + 5,
        )
        return resp is not None and "ready" in resp.get("stdout", "")
    except Exception:
        return False
