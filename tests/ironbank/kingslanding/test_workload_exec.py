"""`POST /vms/{id}/exec` on an image session runs in the workload.

`capsem create --image` leaves a session whose workload runs in a user
namespace (container 0 is VM 100000), under the workload's seccomp filter,
capabilities and cgroup. An untargeted exec must be `runc exec` into that
workload as the image's own user -- not VM root, and not a process entered
from outside with nsenter, which would carry none of the filter. Naming the VM
still reaches the VM, where Capsem's own diagnostics run, and the session
ledger records which side each command ran in, as the caller wrote it.
"""

import base64
import contextlib
import json
import shlex
import struct

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
MCP_LAUNCHER = (
    "chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec "
    + shlex.quote(
        "mount --make-rprivate /; cd /newroot; mount --move . /; mkdir -p run/capsem-image; "
        "mount -t virtiofs -o ro,nosuid,nodev,noexec capsem-image run/capsem-image; "
        "exec chroot . /usr/bin/python3 -I /run/capsem-image/launch.py --mcp-stdio"
    )
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


def mcp_setup(argv, *, workdir=None, variables=None, incoming=b""):
    record = json.dumps({"argv": argv, "workdir": workdir, "variables": variables or {}}).encode()
    return base64.b64encode(struct.pack("!I", len(record)) + record + incoming).decode()


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


def test_mcp_stdio_launcher_preserves_the_workload_user_and_the_first_request(service, tmp_path):
    client = service.client()
    fixture = (
        'IFS= read -r message; printf "UID %s\\nCWD %s\\nVAR %s\\nARG %s\\nRPC %s\\n" '
        '"$(id -u)" "$PWD" "$MCP_FIXTURE" "$1" "$message"'
    )
    literal = "$(echo should-stay-literal)"
    first_request = '{"jsonrpc":"2.0","method":"initialize","id":1}'
    encoded = mcp_setup(
        ["/bin/sh", "-ec", fixture, "fixture", literal], workdir="/workspace",
        variables={"MCP_FIXTURE": "approved"}, incoming=(first_request + "\n").encode(),
    )
    workload = {
        "User": "999:999",
        "WorkingDir": "/tmp",
        "Entrypoint": ["redis-server", "--save", "", "--appendonly", "no"],
    }
    with (
        registry(tmp_path, image_config=workload) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "mcp-workload-launch") as vm,
    ):
        assert run(client, vm["id"], BUNDLE_USER, target="vm").split() == ["999", "/tmp"]
        output = run(client, vm["id"], f"printf %s {encoded} | base64 -d | {MCP_LAUNCHER}", target="vm")
        assert fields(output) == {
            "UID": "999", "CWD": "/workspace", "VAR": "approved", "ARG": literal, "RPC": first_request,
        }


def test_mcp_stdio_launcher_retires_its_cgroup_on_exit_and_cancel(service, tmp_path):
    client = service.client()
    # Keep a separate descendant alive beyond the server's own process. The
    # workload cgroup must survive, while the dedicated MCP leaf must empty.
    fixture = "sleep 60 </dev/null >/dev/null 2>&1 & "
    missing = mcp_setup(["/capsem-mcp-fixture-not-installed"])
    normal = mcp_setup(["/bin/sh", "-ec", fixture + "printf normal; exit 7"])
    long_running = mcp_setup([
        "/bin/sh", "-ec", fixture + "printf 'ready\\n'; printf private-stderr >&2; trap '' TERM; wait",
    ])
    probe = f"""import base64, pathlib, signal, subprocess
root = pathlib.Path('/sys/fs/cgroup/capsem-container')
command = ['/bin/bash', '-c', {('exec ' + MCP_LAUNCHER)!r}]
assert root.is_dir(), root
assert not list(root.glob('mcp-*'))
missing = subprocess.run(command, input=base64.b64decode({missing!r}), capture_output=True, timeout=10)
assert missing.returncode == 255 and not missing.stdout, missing
assert b'check the executable and working directory' in missing.stderr, missing.stderr
assert not list(root.glob('mcp-*'))
normal = subprocess.run(command, input=base64.b64decode({normal!r}), capture_output=True, timeout=10)
assert (normal.returncode, normal.stdout, normal.stderr) == (7, b'normal', b''), normal
assert not list(root.glob('mcp-*'))
with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
    child.stdin.write(base64.b64decode({long_running!r}))
    child.stdin.flush()
    assert child.stdout.readline() == b'ready\\n'
    leaf, = root.glob('mcp-*')
    assert 'populated 1' in (leaf / 'cgroup.events').read_text()
    child.send_signal(signal.SIGTERM)
    stdout, stderr = child.communicate(timeout=10)
    assert (child.returncode, stdout, stderr) == (143, b'', b''), (child.returncode, stdout, stderr)
    assert not leaf.exists()
print('MCP cgroup retired; workload cgroup retained')
"""
    with (
        registry(tmp_path) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "mcp-workload-retirement") as vm,
    ):
        output = run(client, vm["id"], f"python3 -c {shlex.quote(probe)}", target="vm")
        assert output.strip() == "MCP cgroup retired; workload cgroup retained"
        assert run(client, vm["id"], "redis-cli PING").strip() == "PONG"
