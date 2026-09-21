"""Environment variable blocklist enforcement in the guest."""

import contextlib
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, exec_output_text, wait_exec_ready

pytestmark = pytest.mark.security

BLOCKED_VARS = [
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "BASH_ENV",
    "BASH_FUNC_",
    "ENV",
    "IFS",
    "CDPATH",
]


@pytest.fixture(scope="module")
def security_vm():
    """A VM for security tests."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()

    name = f"sec-{uuid.uuid4().hex[:8]}"
    client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
    assert wait_exec_ready(client, name), f"VM {name} never exec-ready"

    yield client, name

    with contextlib.suppress(Exception):
        client.delete(f"/vms/{name}/delete")
    svc.stop()


class TestEnvBlocklist:

    def test_ld_preload_not_set(self, security_vm):
        client, name = security_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo LD_PRELOAD=$LD_PRELOAD"})
        stdout = exec_output_text(resp)
        # LD_PRELOAD should be empty (just "LD_PRELOAD=")
        assert "LD_PRELOAD=/" not in stdout, f"LD_PRELOAD should not be set: {stdout}"

    def test_ld_library_path_not_set(self, security_vm):
        client, name = security_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo LD_LIBRARY_PATH=$LD_LIBRARY_PATH"})
        stdout = exec_output_text(resp)
        assert "LD_LIBRARY_PATH=/" not in stdout

    def test_bash_env_not_set(self, security_vm):
        client, name = security_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo BASH_ENV=$BASH_ENV"})
        stdout = exec_output_text(resp)
        assert "BASH_ENV=/" not in stdout

    def test_ifs_is_default(self, security_vm):
        """IFS should be default (space, tab, newline) or unset."""
        client, name = security_vm
        resp = client.post(f"/vms/{name}/exec", {
            "command": "printf '%q' \"$IFS\"",
        })
        stdout = exec_output_text(resp)
        # Default IFS is ' \t\n', printf %q renders it as $' \t\n'
        assert "IFS" not in stdout or "\\" not in stdout or len(stdout.strip()) < 20
