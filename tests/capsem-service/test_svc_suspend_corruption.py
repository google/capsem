"""Suspend must leave the persistent overlay (rootfs.img) durable on host.

Bug repro: after `capsem suspend <name>` followed by `capsem resume <name>`,
the in-VM `cd /root && ls` fails with "cannot open directory '.':
No such file or directory" -- the EXT4 overlay-upper inode metadata on
rootfs.img is stale because Apple VZ buffered writes via APFS were not
fsync'd before capsem-process exited.

The two assertions below capture both observed failure modes:

1. After a SUCCESSFUL suspend+resume, every file written before suspend
   must still be readable. Today this fails because the overlay reads
   stale inodes.

2. After a successful suspend+resume, a fresh exec of `ls /root` must
   succeed. Today this fails with "cannot open directory" or returns
   nothing because the directory inode itself is corrupt.
"""

import uuid

import pytest
from helpers.constants import (
    CODE_PROFILE_ID,
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
    EXEC_TIMEOUT_SECS,
)
from helpers.service import vm_name, wait_exec_ready

pytestmark = pytest.mark.integration


def _exec(client, name, command):
    return client.post(
        f"/vms/{name}/exec",
        {"command": command, "timeout_secs": EXEC_TIMEOUT_SECS},
    )


class TestSuspendOverlayDurability:

    def test_overlay_files_survive_suspend_resume(self, client):
        """Files on the EXT4 overlay (e.g. /tmp, /etc) must read back cleanly after resume."""
        name = vm_name("ovl")
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

            marker = uuid.uuid4().hex[:8]
            paths = ["/tmp/sus.txt", "/etc/sus.txt", "/var/sus.txt", "/opt/sus.txt"]
            for p in paths:
                w = _exec(client, name, f"mkdir -p $(dirname {p}) && echo {marker} > {p}")
                assert w.get("exit_code") == 0, f"write {p}: {w}"

            sus = client.post(f"/vms/{name}/pause", {})
            assert sus and sus.get("success"), f"suspend failed: {sus}"

            res = client.post(f"/vms/{name}/resume", {})
            assert res is not None, "resume returned None"
            resumed = res.get("id", name)
            assert wait_exec_ready(client, resumed, timeout=EXEC_READY_TIMEOUT), \
                "VM not ready after resume"

            missing = []
            for p in paths:
                r = _exec(client, resumed, f"cat {p} 2>&1")
                if marker not in r.get("stdout", ""):
                    missing.append((p, r.get("exit_code"), r.get("stdout", "")[:200]))
            assert not missing, "overlay files lost after suspend+resume:\n" + "\n".join(
                f"  {p}: exit={ec} out={out!r}" for p, ec, out in missing
            )
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_root_directory_listable_after_suspend_resume(self, client):
        """`ls /root` must succeed after suspend+resume (the bug repro)."""
        name = vm_name("lsroot")
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

            # Touch a file so /root has something with a known inode.
            _exec(client, name, "echo hello > /root/before.txt")

            sus = client.post(f"/vms/{name}/pause", {})
            assert sus and sus.get("success"), f"suspend failed: {sus}"

            res = client.post(f"/vms/{name}/resume", {})
            assert res is not None
            resumed = res.get("id", name)
            assert wait_exec_ready(client, resumed, timeout=EXEC_READY_TIMEOUT)

            r = _exec(client, resumed, "cd /root && ls -la")
            assert r.get("exit_code") == 0, (
                f"`cd /root && ls -la` failed after resume: exit={r.get('exit_code')} "
                f"stdout={r.get('stdout')!r} stderr={r.get('stderr')!r}"
            )
            assert "before.txt" in r.get("stdout", ""), \
                f"before.txt missing after resume: {r}"

            # Qualification once caught a subtler EXT4 inode failure where
            # lstat(2) still saw this fast symlink but readlink(2) returned
            # ENOENT.  `ls -la` reported the corrupt entry and exited 1. Keep
            # the exact signature in the runtime regression, not only the
            # broader directory-listability assertion above.
            link = _exec(client, resumed, "readlink /root/.venv")
            assert link.get("exit_code") == 0, \
                f".venv readlink failed after resume: {link}"
            assert link.get("stdout", "").strip() == "/run/capsem-venv", \
                f".venv target corrupted after resume: {link}"
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_open_workspace_file_and_directory_fds_survive_suspend_resume(self, client):
        """Warm resume preserves pre-existing VirtioFS file and directory handles."""
        name = vm_name("openfd")
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
            setup = _exec(
                client,
                name,
                r"""set -eu
rm -f /tmp/capsem-fd-ready /tmp/capsem-fd-go /tmp/capsem-fd-done
mkfifo /tmp/capsem-fd-ready /tmp/capsem-fd-go /tmp/capsem-fd-done
printf 'before\n' > /root/held-open.txt
cat > /tmp/capsem-hold-fds.py <<'PY'
import os

held = open('/root/held-open.txt', 'a', encoding='utf-8')
directory = os.open('/root', os.O_RDONLY | os.O_DIRECTORY)
with open('/tmp/capsem-fd-ready', 'w', encoding='utf-8') as ready:
    ready.write('ready\n')
with open('/tmp/capsem-fd-go', encoding='utf-8') as go:
    assert go.readline().strip() == 'go'
held.write('after\n')
held.flush()
os.fsync(held.fileno())
entries = sorted(os.listdir(directory))
with open('/tmp/capsem-fd-done', 'w', encoding='utf-8') as done:
    done.write(','.join(entries) + '\n')
PY
setsid python3 /tmp/capsem-hold-fds.py </dev/null >/tmp/capsem-hold-fds.log 2>&1 &
IFS= read -r ready < /tmp/capsem-fd-ready
test "$ready" = ready""",
            )
            assert setup.get("exit_code") == 0, f"open-FD setup failed: {setup}"

            suspended = client.post(f"/vms/{name}/pause", {})
            assert suspended and suspended.get("success"), f"suspend failed: {suspended}"
            response = client.post(f"/vms/{name}/resume", {})
            assert response is not None, "resume returned None"
            resumed = response.get("id", name)
            assert wait_exec_ready(client, resumed, timeout=EXEC_READY_TIMEOUT)

            result = _exec(
                client,
                resumed,
                """set -eu
printf 'go\n' > /tmp/capsem-fd-go
IFS= read -r entries < /tmp/capsem-fd-done
printf '%s\n' "$entries"
cat /root/held-open.txt""",
            )
            assert result.get("exit_code") == 0, f"restored open FDs failed: {result}"
            lines = result.get("stdout", "").splitlines()
            assert "held-open.txt" in lines[0].split(","), result
            assert lines[1:] == ["before", "after"], result
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_suspend_failure_does_not_brick_vm(self, client):
        """Heavy-overlay write + suspend + resume + suspend + resume.

        The bug surfaces when EXT4 metadata is dirtied across many inodes
        before suspend; if Apple VZ's writes to the host rootfs.img file
        aren't fsync'd before capsem-process exits, APFS may serve stale
        bytes to the next boot, and `iget: checksum invalid` panics the
        guest on overlay mount.
        """
        name = vm_name("brick")
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

            # Generate sustained metadata churn on the overlay (rootfs.img).
            # Many small files across many directories => many dirty inodes.
            churn = (
                "set -e; for d in /tmp /var/log /opt /usr/local /etc; do "
                "  for i in $(seq 1 100); do echo data$i > $d/churn_${RANDOM}_$i.txt; done; "
                "done; sync"
            )
            r = _exec(client, name, churn)
            assert r.get("exit_code") == 0, f"churn failed: {r}"

            for cycle in range(3):
                sus = client.post(f"/vms/{name}/pause", {})
                assert sus and sus.get("success"), f"[cycle {cycle}] suspend failed: {sus}"

                res = client.post(f"/vms/{name}/resume", {})
                assert res is not None, f"[cycle {cycle}] resume returned None"
                resumed = res.get("id", name)
                assert wait_exec_ready(client, resumed, timeout=EXEC_READY_TIMEOUT), \
                    f"[cycle {cycle}] VM bricked after suspend+resume (overlay corruption)"

                # Quick health probe -- if overlay is corrupt this fails.
                r = _exec(client, resumed, "ls /etc/churn_* 2>&1 | head -1 && cd /root && ls > /dev/null")
                assert r.get("exit_code") == 0, \
                    f"[cycle {cycle}] post-resume health probe failed: {r}"
        finally:
            client.delete(f"/vms/{name}/delete")
