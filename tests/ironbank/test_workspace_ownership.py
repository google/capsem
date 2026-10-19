"""VM metadata reads cannot revoke a workload's workspace permissions."""

from __future__ import annotations

import shlex
import sys
import time
import uuid
from concurrent.futures import ThreadPoolExecutor

import pytest
from helpers.image_session import workload_exec

from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv

pytestmark = [pytest.mark.integration, pytest.mark.skipif(sys.platform != "linux", reason="KVM VirtioFS ownership")]


def test_vm_metadata_reads_preserve_unprivileged_workspace_writes(model_client_env: ModelClientEnv) -> None:
    env = model_client_env
    marker = f"/tmp/workspace-owner-{uuid.uuid4().hex}"
    reader = (
        "import os,time; from pathlib import Path; assert os.getuid()==0; "
        f"Path({marker!r}).touch(); deadline=time.monotonic()+15\n"
        "while time.monotonic()<deadline: os.stat('/root')\n"
        "print('VM_METADATA_READS_COMPLETE')"
    )
    writer = """import os,subprocess
from pathlib import Path
assert os.getuid()==1000
assert os.stat('/workspace').st_mode & 0o002 == 0
for index in range(500):
    path='/workspace/owner-race-'+str(index)
    result=subprocess.run(['/bin/sh','-c','umask 077; printf test > '+path],capture_output=True,text=True)
    assert result.returncode==0, (index,result.stderr)
    assert os.stat(path).st_mode & 0o777 == 0o600
    assert Path(path).read_text()=='test'
    Path(path).unlink()
print('WORKSPACE_WRITES_COMPLETE')
"""

    def read_metadata() -> dict:
        return env.client.post(f"/vms/{env.session_id}/exec", {
            "command": "python3 -c " + shlex.quote(reader), "target": "vm", "timeout_secs": 25,
        }, timeout=35)

    with ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(read_metadata)
        try:
            deadline = time.monotonic() + 10
            while True:
                ready = env.client.post(f"/vms/{env.session_id}/exec", {
                    "command": "test -f " + marker, "target": "vm", "timeout_secs": 2,
                }, timeout=5)
                if ready["exit_code"] == 0:
                    break
                assert time.monotonic() < deadline, "VM metadata reader did not start"
            assert not future.done(), "metadata reads must overlap workload writes"
            result = workload_exec(env.client, env.session_id, "python3 -c " + shlex.quote(writer), timeout=30)
            assert result["exit_code"] == 0, result
        finally:
            result = future.result(timeout=35)
            assert result["exit_code"] == 0, result
