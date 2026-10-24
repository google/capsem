"""Resume persistence across filesystem paths.

The /root tree is bind-mounted from the host VirtioFS workspace -- files there
are preserved because the host owns the directory. The persistent guest
filesystem lives on the overlayfs upper layer (rootfs.img attached as
virtio-blk /dev/vdb). Runtime directories use fresh tmpfs on cold boot;
a warm checkpoint preserves their memory instead.

Regression net for "file disappears after stop+resume" reports: write a marker
to several representative paths via shell exec (the way a user would), stop
the VM, resume, and assert each marker is still readable.
"""

import uuid

import pytest
from helpers.constants import (
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
    EXEC_TIMEOUT_SECS,
)
from helpers.service import exec_output_text, vm_name, wait_exec_ready

pytestmark = pytest.mark.integration


# Directories spanning workspace bind-mount and overlayfs upper.
# /root              -> VirtioFS workspace bind-mount
# /etc, /var, /opt, /usr/local -> overlayfs upper on rootfs.img
PERSIST_PATHS = [
    "/root",
    "/etc",
    "/var",
    "/opt",
    "/usr/local",
]
RUNTIME_PATHS = ["/run", "/tmp", "/var/log"]


def _exec(client, name, command):
    return client.post(
        f"/vms/{name}/exec",
        {"command": command, "timeout_secs": EXEC_TIMEOUT_SECS},
    )


def _transition(client, name, action):
    status, response = client.call_json("POST", f"/vms/{name}/{action}", {})
    assert status == 200, response
    return response


class TestResumePathPersistence:

    def _paths_for(self, marker, paths=PERSIST_PATHS):
        return [f"{base}/marker-{marker}.txt" for base in paths]

    def _write_markers(self, client, name, marker):
        for path in self._paths_for(marker, PERSIST_PATHS + RUNTIME_PATHS):
            resp = _exec(
                client,
                name,
                f"mkdir -p $(dirname {path}) && echo {marker} > {path} && cat {path}",
            )
            assert resp.get("exit_code") == 0, f"write to {path} failed: {resp}"
            assert exec_output_text(resp) == f"{marker}\n", \
                f"write/read-back of {path} did not see marker: {resp}"

    def _check_markers(self, client, name, marker, *, warm=False):
        missing = []
        paths = PERSIST_PATHS + RUNTIME_PATHS if warm else PERSIST_PATHS
        for path in self._paths_for(marker, paths):
            resp = _exec(client, name, f"cat {path} 2>&1")
            stdout = exec_output_text(resp)
            if resp.get("exit_code") != 0 or stdout != f"{marker}\n":
                missing.append((path, resp.get("exit_code"), stdout.strip()[:200]))
        return missing

    def _check_runtime_reset(self, client, name, marker):
        for path in self._paths_for(marker, RUNTIME_PATHS):
            resp = _exec(client, name, f"test ! -e {path}")
            assert resp.get("exit_code") == 0 and exec_output_text(resp) == "", (
                f"runtime marker survived cold boot at {path}: {resp}"
            )

    def test_missing_marker_path_cannot_satisfy_persistence(self, client):
        """An error containing the marker in its filename is not retained data."""
        name = vm_name("missing")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            marker = f"absent-{uuid.uuid4().hex[:8]}"
            missing = self._check_markers(client, name, marker)
            assert {path for path, _, _ in missing} == set(self._paths_for(marker))
            assert all(code != 0 for _, code, _ in missing), missing
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_files_survive_stop_resume_across_paths(self, client):
        """Write marker files to overlay + workspace paths, stop, resume, verify all survive."""
        name = vm_name("paths")
        client.post(
            "/vms/create",
            {
                "name": name,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
            },
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
                f"VM {name} never became exec-ready"

            marker = f"persist-{uuid.uuid4().hex[:8]}"
            self._write_markers(client, name, marker)

            # Stop the VM (preserves state for persistent VMs).
            _transition(client, name, "stop")

            # Resume.
            resume_resp = _transition(client, name, "resume")
            assert resume_resp is not None, "resume returned None"
            resumed_id = resume_resp.get("id", name)
            assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
                f"VM {resumed_id} never became exec-ready after resume"

            missing = self._check_markers(client, resumed_id, marker)
            assert not missing, (
                "Files lost after stop+resume:\n"
                + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
            )
            self._check_runtime_reset(client, resumed_id, marker)
        finally:
            client.delete(f"/vms/{name}/delete")

    @pytest.mark.skip(reason="Warm suspend disabled for 0.7.1; original warm persistence acceptance retained for P0 #378")
    def test_files_survive_suspend_resume_across_paths(self, client):
        """Same coverage as the stop test, but using the warm suspend/resume path."""
        name = vm_name("susp")
        client.post(
            "/vms/create",
            {
                "name": name,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
            },
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
                f"VM {name} never became exec-ready"

            marker = f"suspend-{uuid.uuid4().hex[:8]}"
            self._write_markers(client, name, marker)

            # Suspend (warm checkpoint via Apple VZ saveMachineState).
            _transition(client, name, "pause")

            # Resume (restores from checkpoint).
            resume_resp = _transition(client, name, "resume")
            assert resume_resp is not None, "resume returned None"
            resumed_id = resume_resp.get("id", name)
            assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
                f"VM {resumed_id} never became exec-ready after suspend+resume"

            missing = self._check_markers(client, resumed_id, marker, warm=True)
            assert not missing, (
                "Files lost after suspend+resume:\n"
                + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
            )
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_files_survive_back_to_back_stop_resume(self, client):
        """Two stop/resume cycles on the same VM, accumulating writes."""
        name = vm_name("backtoback")
        client.post(
            "/vms/create",
            {
                "name": name,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
            },
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

            marker_a = f"cycle-a-{uuid.uuid4().hex[:6]}"
            self._write_markers(client, name, marker_a)
            _transition(client, name, "stop")
            _transition(client, name, "resume")
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            assert not self._check_markers(client, name, marker_a), \
                "first resume lost files written before first stop"
            self._check_runtime_reset(client, name, marker_a)

            marker_b = f"cycle-b-{uuid.uuid4().hex[:6]}"
            self._write_markers(client, name, marker_b)
            _transition(client, name, "stop")
            _transition(client, name, "resume")
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            # Both A (from before first stop) and B (from before second stop)
            # must still be there.
            for marker in (marker_a, marker_b):
                missing = self._check_markers(client, name, marker)
                assert not missing, (
                    f"second resume lost {marker}:\n"
                    + "\n".join(f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing)
                )
                self._check_runtime_reset(client, name, marker)
        finally:
            client.delete(f"/vms/{name}/delete")
