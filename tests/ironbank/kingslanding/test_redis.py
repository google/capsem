"""Explicit offline Redis spike; prefetch with tests/fixtures/oci/prepare_redis.py.

The owning gate prepares the native image before hermetic execution.
Loopback-only Redis traffic never enters Capsem's host network ledger.
"""

import contextlib
import hashlib
import json
import sqlite3
from pathlib import Path

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_name, vm_session_db_path, wait_exec_ready

from tests.fixtures.oci.prepare_redis import native_pin
from tests.ironbank.kingslanding.test_oci_container import FIXTURES, oci_vm

__all__ = ["oci_vm"]

IMAGE = Path(__file__).resolve().parents[3] / "cache/target/tests/redis-image"
# Capsem boots its agents in a chroot. Give this fixture a private mount
# namespace whose root is the guest root, so runc exec joins the correct root.
PROBE_COMMAND = (
    "chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec "
    "'mount --make-rprivate /; cd /newroot; mount --move . /; "
    "exec chroot . /usr/bin/python3 /root/oci/redis_probe.py'"
)


def test_real_redis_persistence_limits_and_fresh_vm(oci_vm, tmp_path):
    service, client, first = oci_vm
    pin = native_pin()
    metadata = json.loads((IMAGE / "redis-image.json").read_text())
    assert {key: metadata[key] for key in pin} == pin
    archive = (IMAGE / "redis-rootfs.tar.gz").read_bytes()
    assert hashlib.sha256(archive).hexdigest() == metadata["archive_sha256"]
    second = vm_name("redis-fresh")
    client.post(
        "/vms/create",
        {
            "name": second,
            "profile_id": CODE_PROFILE_ID,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        },
        timeout=90,
    )
    try:
        assert wait_exec_ready(client, second)
        for index, name in enumerate((first, second)):
            check = {
                "command": "readlink /proc/self/ns/mnt; test -f /proc/1/root/init",
                "timeout_secs": 10,
            }
            before = client.post(f"/vms/{name}/exec", check)
            assert before["exit_code"] == 0, before
            for filename, contents in {
                "bundle.py": (FIXTURES / "bundle.py").read_bytes(),
                "redis_probe.py": (FIXTURES / "redis_probe.py").read_bytes(),
                "redis-image.json": (IMAGE / "redis-image.json").read_bytes(),
                **{
                    f"redis-rootfs.part{index:03d}": archive[
                        offset : offset + 1024 * 1024
                    ]
                    for index, offset in enumerate(range(0, len(archive), 1024 * 1024))
                },
            }.items():
                assert client.post_bytes(
                    f"/vms/{name}/files/content?path=oci/{filename}", contents
                ) == {
                    "success": True,
                    "size": len(contents),
                }
            result = client.post(
                f"/vms/{name}/exec",
                {
                    "command": PROBE_COMMAND,
                    "timeout_secs": 180,
                },
                timeout=200,
            )
            (tmp_path / f"redis-session-{index}.json").write_text(
                json.dumps(result, indent=2)
            )
            assert result["exit_code"] == 0, result
            assert client.post(f"/vms/{name}/exec", check) == before
            status, raw = client.get_bytes(
                f"/vms/{name}/files/content?path=oci/redis-evidence.json"
            )
            assert status == 200
            evidence = json.loads(raw)
            assert evidence["image"] == pin["image"]
            assert evidence["archive_sha256"] == metadata["archive_sha256"]
            (tmp_path / f"redis-evidence-{index}.json").write_bytes(raw)
            if index == 0:
                first_result = result
                first_rdb = evidence["rdb_sha256"]
        # The first VM still has its RDB while the fresh VM has run independently.
        retained = client.post(
            f"/vms/{first}/exec",
            {
                "command": "sha256sum /var/tmp/redis-session-proof.rdb",
                "timeout_secs": 10,
            },
        )
        assert retained["exit_code"] == 0 and retained["stdout"].split()[0] == first_rdb
    finally:
        client.delete(f"/vms/{second}/delete")
    db_path = vm_session_db_path(service.tmp_dir, client, first)
    service.stop(cleanup=False)
    with contextlib.closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as db:
        rows = db.execute(
            "SELECT exit_code, source, stdout_bytes, stderr_bytes, credential_ref "
            "FROM exec_events WHERE command = ?",
            (PROBE_COMMAND,),
        ).fetchall()
    assert rows == [
        (
            0,
            "api",
            len(first_result["stdout"].encode()),
            len(first_result["stderr"].encode()),
            None,
        )
    ]
