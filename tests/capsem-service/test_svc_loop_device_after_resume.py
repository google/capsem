"""System-overlay EXT4 must remain healthy across suspend/resume.

Closes sprints/done/virtio-blk-overlay-migration/ISSUE.md (which
itself closes sprints/done/loop-device-io-after-resume/ISSUE.md).

Pre-virtio-blk-migration the guest mounted the system overlay through
a loop device on top of a VirtioFS-served file. After heavy directory
churn + suspend/resume the kernel logged

    EXT4-fs (loop0): failed to convert unwritten extents to written
    extents -- potential data loss!
    I/O error, dev loop0, sector ... op 0x1:(WRITE)

because Apple VZ's closed-source virtiofsd EIOs under writeback
pressure on resume. The fix moves the overlay onto a real virtio-blk
device (/dev/vdb), eliminating the layer that was returning EIO.

The test reproduces the heavy-churn pattern, suspends, resumes, and
asserts no NEW ext4 errors -- on either the legacy loop0 (must never
appear post-migration) or the new vdb -- show up in dmesg.
"""

import re
import uuid

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT, EXEC_TIMEOUT_SECS
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


# Any ext4-level error on the system-overlay device, plus the legacy
# loop0 signatures (which must never reappear after migration).
LOOP_ERROR_PATTERNS = [
    re.compile(r"EXT4-fs \((loop0|vdb)\): failed to convert unwritten extents"),
    re.compile(r"I/O error, dev (loop0|vdb), sector \d+ op 0x1:\(WRITE\)"),
    re.compile(r"EXT4-fs error \(device (loop0|vdb)\): .*iget: checksum invalid"),
    re.compile(r"Aborting journal on device (loop0|vdb)"),
]


def _exec(client, name, command):
    return client.post(
        f"/vms/{name}/exec",
        {"command": command, "timeout_secs": EXEC_TIMEOUT_SECS},
    )


def _dmesg_offending_lines(client, name):
    """Return dmesg lines from the guest that match the bug signatures."""
    resp = _exec(client, name, "dmesg")
    out = resp.get("stdout", "")
    matches = []
    for line in out.splitlines():
        for pat in LOOP_ERROR_PATTERNS:
            if pat.search(line):
                matches.append(line.strip())
                break
    return matches


class TestLoopDeviceAfterResume:

    def test_dmesg_clean_after_heavy_churn_suspend_resume(self, client):
        """Heavy directory churn + suspend + resume must NOT leave EXT4 errors in dmesg.

        Closed by virtio-blk-overlay-migration: moving the system overlay
        off loop-on-VirtioFS onto a real virtio-blk device eliminated
        the writeback-pressure EIO that produced these warnings.
        """
        name = vm_name("loopio")
        client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "persistent": True,
            },
        )
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

            # Snapshot the pre-suspend dmesg state so we only count NEW
            # errors after resume. dmesg -C requires CAP_SYSLOG which is
            # unreliable here, so we diff strings instead.
            pre = set(_dmesg_offending_lines(client, name))

            # Reproduce the churn pattern: 50 small files per overlay dir
            # across five dirs = 250 new directory entries spread across
            # five inodes' dirent blocks. This shape hit the bug in
            # manual MCP testing.
            tag = uuid.uuid4().hex[:6]
            churn = (
                "set -e; "
                f"for d in /tmp /var /opt /etc /usr/local; do "
                f"  for i in $(seq 1 50); do "
                f"    echo data-{tag}-$i > $d/churn_${{RANDOM}}_${{i}}_{tag}; "
                f"  done; "
                f"done; sync"
            )
            r = _exec(client, name, churn)
            assert r.get("exit_code") == 0, f"churn write failed: {r}"

            sus = client.post(f"/vms/{name}/pause", {})
            assert sus and sus.get("success"), f"suspend failed: {sus}"

            res = client.post(f"/vms/{name}/resume", {})
            assert res is not None, "resume returned None"
            resumed = res.get("id", name)
            assert wait_exec_ready(client, resumed, timeout=EXEC_READY_TIMEOUT), \
                "VM not ready after resume"

            # Touch the directories so the guest tries to flush the
            # cached metadata back to the overlay device. Without this
            # the error path may not fire until later use.
            _exec(client, resumed, "ls /tmp /etc /var /opt /usr/local > /dev/null 2>&1; sync")

            post = _dmesg_offending_lines(client, resumed)
            new_errors = [l for l in post if l not in pre]
            assert not new_errors, (
                "System-overlay EXT4 errors NEW after suspend/resume "
                "(see sprints/done/virtio-blk-overlay-migration/ISSUE.md):\n"
                + "\n".join(f"  {l}" for l in new_errors[:10])
            )
        finally:
            client.delete(f"/vms/{name}/delete")
