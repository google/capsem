"""Resource bound enforcement: CPU and RAM limits."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance

pytestmark = pytest.mark.config


@pytest.fixture(scope="module")
def config_svc():
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()


class TestCpuLimits:

    def test_cpu_zero_rejected(self, config_svc):
        client = config_svc.client()
        name = f"cpu0-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": 0})
        assert resp is None or "error" in str(resp).lower(), f"cpus=0 should be rejected: {resp}"
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass

    def test_cpu_over_max_rejected(self, config_svc):
        client = config_svc.client()
        name = f"cpumax-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": 99})
        assert resp is None or "error" in str(resp).lower(), f"cpus=99 should be rejected: {resp}"
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass

    def test_cpu_valid_accepted(self, config_svc):
        client = config_svc.client()
        name = f"cpuok-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": 4})
        assert resp is not None
        client.delete(f"/delete/{name}")


class TestRamLimits:

    def test_ram_zero_rejected(self, config_svc):
        client = config_svc.client()
        name = f"ram0-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": 0, "cpus": DEFAULT_CPUS})
        assert resp is None or "error" in str(resp).lower(), f"ram=0 should be rejected: {resp}"
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass

    def test_ram_over_max_rejected(self, config_svc):
        client = config_svc.client()
        name = f"rammax-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": 999999, "cpus": DEFAULT_CPUS})
        assert resp is None or "error" in str(resp).lower(), f"ram=999999 should be rejected: {resp}"
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass

    def test_ram_valid_accepted(self, config_svc):
        client = config_svc.client()
        name = f"ramok-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": 4096, "cpus": DEFAULT_CPUS})
        assert resp is not None
        client.delete(f"/delete/{name}")
