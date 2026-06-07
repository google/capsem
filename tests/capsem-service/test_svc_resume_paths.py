"""Resume persistence across filesystem paths.

The /root tree is bind-mounted from the host VirtioFS workspace -- files there
are obviously preserved because the host owns the directory. The rest of the
guest filesystem lives on the overlayfs upper layer (rootfs.img attached
as virtio-blk /dev/vdb), which is also persisted in the session_dir on
the host.

Regression net for "file disappears after stop+resume" reports: write a marker
to several representative paths via shell exec (the way a user would), stop
the VM, resume, and assert each marker is still readable.
"""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT, EXEC_TIMEOUT_SECS
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


# Directories spanning workspace bind-mount and overlayfs upper.
# /root              -> VirtioFS workspace bind-mount
# /tmp, /etc, /var, /opt, /usr/local -> overlayfs upper on rootfs.img
PERSIST_PATHS = [
    "/root",
    "/tmp",
    "/etc",
    "/var",
    "/opt",
    "/usr/local",
]


def _exec(client, name, command):
    return client.post(
        f"/exec/{name}",
        {"command": command, "timeout_secs": EXEC_TIMEOUT_SECS},
    )


class TestResumePathPersistence:

    def _paths_for(self, marker):
        return [f"{base}/marker-{marker}.txt" for base in PERSIST_PATHS]

    def _write_markers(self, client, name, marker):
        for path in self._paths_for(marker):
            resp = _exec(
                client,
                name,
                f"mkdir -p $(dirname {path}) && echo {marker} > {path} && cat {path}",
            )
            assert resp.get("exit_code") == 0, f"write to {path} failed: {resp}"
            assert marker in resp.get("stdout", ""), \
                f"write/read-back of {path} did not see marker: {resp}"

    def _check_markers(self, client, name, marker):
        missing = []
        for path in self._paths_for(marker):
            resp = _exec(client, name, f"cat {path} 2>&1")
            stdout = resp.get("stdout", "")
            if marker not in stdout:
                missing.append((path, resp.get("exit_code"), stdout.strip()[:200]))
        return missing

    def test_files_survive_stop_resume_across_paths(self, client):
        """Write marker files to overlay + workspace paths, stop, resume, verify all survive."""
        name = vm_name("paths")
        client.post(
            "/provision",
            {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True},
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
                f"VM {name} never became exec-ready"

            marker = f"persist-{uuid.uuid4().hex[:8]}"
            self._write_markers(client, name, marker)

            # Stop the VM (preserves state for persistent VMs).
            client.post(f"/stop/{name}", {})

            # Resume.
            resume_resp = client.post(f"/resume/{name}", {})
            assert resume_resp is not None, "resume returned None"
            resumed_id = resume_resp.get("id", name)
            assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
                f"VM {resumed_id} never became exec-ready after resume"

            missing = self._check_markers(client, resumed_id, marker)
            assert not missing, (
                "Files lost after stop+resume:\n"
                + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
            )
        finally:
            client.delete(f"/delete/{name}")

    def test_files_survive_suspend_resume_across_paths(self, client):
        """Same coverage as the stop test, but using the warm suspend/resume path."""
        name = vm_name("susp")
        client.post(
            "/provision",
            {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True},
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
                f"VM {name} never became exec-ready"

            marker = f"suspend-{uuid.uuid4().hex[:8]}"
            self._write_markers(client, name, marker)

            # Suspend (warm checkpoint via Apple VZ saveMachineState).
            client.post(f"/suspend/{name}", {})

            # Resume (restores from checkpoint).
            resume_resp = client.post(f"/resume/{name}", {})
            assert resume_resp is not None, "resume returned None"
            resumed_id = resume_resp.get("id", name)
            assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
                f"VM {resumed_id} never became exec-ready after suspend+resume"

            missing = self._check_markers(client, resumed_id, marker)
            assert not missing, (
                "Files lost after suspend+resume:\n"
                + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
            )
        finally:
            client.delete(f"/delete/{name}")

    def test_files_survive_back_to_back_stop_resume(self, client):
        """Two stop/resume cycles on the same VM, accumulating writes."""
        name = vm_name("backtoback")
        client.post(
            "/provision",
            {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True},
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

            marker_a = f"cycle-a-{uuid.uuid4().hex[:6]}"
            self._write_markers(client, name, marker_a)
            client.post(f"/stop/{name}", {})
            client.post(f"/resume/{name}", {})
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            assert not self._check_markers(client, name, marker_a), \
                "first resume lost files written before first stop"

            marker_b = f"cycle-b-{uuid.uuid4().hex[:6]}"
            self._write_markers(client, name, marker_b)
            client.post(f"/stop/{name}", {})
            client.post(f"/resume/{name}", {})
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            # Both A (from before first stop) and B (from before second stop)
            # must still be there.
            for marker in (marker_a, marker_b):
                missing = self._check_markers(client, name, marker)
                assert not missing, (
                    f"second resume lost {marker}:\n"
                    + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
                )
        finally:
            client.delete(f"/delete/{name}")
