"""Gateway concurrency and stress tests.

Verifies the gateway handles parallel requests correctly through the
real binary process (not just Rust unit tests).
"""

import json
import subprocess
import threading
import time

import pytest

pytestmark = pytest.mark.gateway


class TestConcurrentRequests:

    def test_parallel_list_requests(self, gateway_env, gw_client):
        """10 concurrent GET /list requests all succeed."""
        results = []
        errors = []

        def do_list():
            try:
                resp = gw_client.get("/list", timeout=10)
                results.append(resp)
            except Exception as e:
                errors.append(str(e))

        threads = [threading.Thread(target=do_list) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=15)

        assert len(errors) == 0, f"concurrent requests failed: {errors}"
        assert len(results) == 10
        for resp in results:
            assert resp is not None
            assert "sandboxes" in resp

    def test_parallel_mixed_endpoints(self, gateway_env, gw_client):
        """Concurrent requests to different endpoints don't interfere."""
        results = {}
        errors = []

        def do_request(name, method, path, body=None):
            try:
                if method == "GET":
                    resp = gw_client.get(path, timeout=10)
                elif method == "POST":
                    resp = gw_client.post(path, body, timeout=10)
                elif method == "DELETE":
                    resp = gw_client.delete(path, timeout=10)
                results[name] = resp
            except Exception as e:
                errors.append(f"{name}: {e}")

        threads = [
            threading.Thread(target=do_request, args=("list", "GET", "/list")),
            threading.Thread(target=do_request, args=("status", "GET", "/status")),
            threading.Thread(target=do_request, args=("info", "GET", "/info/vm-001")),
            threading.Thread(target=do_request, args=("images", "GET", "/images")),
            threading.Thread(target=do_request, args=("provision", "POST", "/provision", {"ram_mb": 2048, "cpus": 2})),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=15)

        assert len(errors) == 0, f"mixed concurrent requests failed: {errors}"
        assert "list" in results
        assert "status" in results
        assert "info" in results

    def test_status_under_concurrent_load(self, gateway_env, gw_client):
        """Multiple concurrent /status requests hit the cache correctly.

        The status cache has a 2s TTL. Rapid concurrent requests should
        all be served from the same cache entry (at most 1 upstream fetch).
        """
        results = []
        errors = []

        def do_status():
            try:
                resp = gw_client.get("/status", timeout=10)
                results.append(resp)
            except Exception as e:
                errors.append(str(e))

        threads = [threading.Thread(target=do_status) for _ in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=15)

        assert len(errors) == 0, f"concurrent status requests failed: {errors}"
        assert len(results) == 20
        # All should have the same vm_count (served from cache)
        counts = set(r["vm_count"] for r in results if r)
        assert len(counts) == 1, f"cache inconsistency: got different vm_counts: {counts}"
