"""`POST /vms/{id}/exec` on an image session runs in the workload.

`capsem create --image` leaves a session whose workload runs in a user
namespace (container 0 is VM 100000), under the workload's seccomp filter,
capabilities and cgroup. An untargeted exec must be `runc exec` into that
workload as the image's own user -- not VM root, and not a process entered
from outside with nsenter, which would carry none of the filter. Naming the VM
still reaches the VM, where Capsem's own diagnostics run, and the session
ledger records which side each command ran in, as the caller wrote it.
"""

import contextlib

import pytest
from helpers.service import exec_output_text, vm_session_db_path
from helpers.session_ledger import open_session_ledger

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

IN_WORKLOAD = (
    'echo "MAP $(cat /proc/self/uid_map)"; '
    'echo "UID $(id -u)"; '
    'echo "CWD $(pwd)"; '
    'if unshare -U true 2>/dev/null; then echo "UNSHARE allowed"; else echo "UNSHARE denied"; fi'
)
IN_VM = "test -f /var/tmp/capsem-container/workload.pid && echo VM-$(id -u)"
# The user the image's process runs as, from the bundle the launcher wrote
# under the image digest's unpacked root (only the current digest's is kept).
BUNDLE_USER = (
    "python3 -c 'import glob, json; [c] = glob.glob(\"/var/lib/capsem/roots/*/bundle/config.json\"); "
    'p = json.load(open(c))[\"process\"]; print(p["user"]["uid"], p["cwd"])\''
)


def run(client, vm_id, command, target=None):
    body = {"command": command, "timeout_secs": 30}
    if target is not None:
        body["target"] = target
    result = client.post(f"/vms/{vm_id}/exec", body, timeout=40)
    assert result.get("exit_code") == 0, result
    return exec_output_text(result)


def fields(output):
    return {line.split(" ", 1)[0]: line.split(" ", 1)[1] for line in output.splitlines() if " " in line}


def test_an_image_session_exec_enters_the_workload_and_the_vm_on_request(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "workload-exec") as vm,
    ):
        uid, cwd = run(client, vm["id"], BUNDLE_USER, target="vm").split()

        inside = fields(run(client, vm["id"], IN_WORKLOAD))
        (tmp_path / "workload.txt").write_text(repr(inside))
        assert inside["MAP"].split() == ["0", "100000", "65536"], inside
        assert inside["UID"] == uid, inside
        assert inside["CWD"] == cwd, inside
        # The workload's seccomp filter applies to the exec, not only to
        # the process the launcher started.
        assert inside["UNSHARE"] == "denied", inside

        # The same request naming the VM sees the VM: the runtime state
        # the workload cannot see, as VM root.
        assert run(client, vm["id"], IN_VM, target="vm").strip() == "VM-0"
        # An unknown target is refused, never read as the default.
        status, refused = client.call_json(
            "POST", f"/vms/{vm['id']}/exec", {"command": "true", "target": "container", "timeout_secs": 5}
        )
        assert 400 <= status < 500, (status, refused)

        client.post(f"/vms/{vm['id']}/stop", {}, timeout=60)
        db_path = vm_session_db_path(service.tmp_dir, client, vm["id"])
        with contextlib.closing(open_session_ledger(db_path)) as db:
            rows = db.execute(
                "SELECT command, target, exit_code FROM exec_events WHERE command IN (?, ?, ?) ORDER BY id",
                (BUNDLE_USER, IN_WORKLOAD, IN_VM),
            ).fetchall()
    # The ledger keeps what the caller asked for, never the launcher wrapper.
    assert rows == [(BUNDLE_USER, "vm", 0), (IN_WORKLOAD, "workload", 0), (IN_VM, "vm", 0)], rows
