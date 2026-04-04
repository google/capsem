"""Shared fixtures for capsem CLI integration tests."""

import subprocess

import pytest

from pathlib import Path

from helpers.service import ServiceInstance

PROJECT_ROOT = Path(__file__).parent.parent.parent
CLI_BINARY = PROJECT_ROOT / "target/debug/capsem"

pytestmark = pytest.mark.integration


def run_cli(*args, uds_path=None, timeout=60):
    """Run capsem CLI and return (stdout, stderr, returncode)."""
    cmd = [str(CLI_BINARY)]
    if uds_path:
        cmd += ["--uds-path", str(uds_path)]
    cmd += list(args)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return result.stdout, result.stderr, result.returncode


@pytest.fixture(scope="session")
def service_env():
    """Start a capsem-service on an isolated temp socket."""
    svc = ServiceInstance()
    svc.start()
    yield svc
    svc.stop()


@pytest.fixture
def uds_path(service_env):
    return service_env.uds_path
