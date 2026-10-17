"""The workspace behaves like a filesystem root can use, without host ownership.

/root is the VirtioFS workspace. The guest sees every entry owned by 0:0 and
never changes host ownership, so a guest chown is accepted without effect. It
used to reach an unprivileged host lchown and fail with EPERM, which broke
`cp -a`, `tar -x` and any seed copy into /root.

cp -a sets the final mode through a POSIX ACL xattr. Apple's VirtioFS server
stores that xattr without applying it, so on macOS a copied 0640 file stayed
at 0600 until the guest kernel learned to refuse ACLs for such servers
(config/docker/image/kernel/patches/0001-*).
"""

import pytest
from helpers.service import exec_output_text

pytestmark = pytest.mark.serial


def _run(client, name: str, command: str) -> tuple[int, str]:
    resp = client.post(f"/vms/{name}/exec", {"command": command, "timeout_secs": 60})
    assert resp is not None, command
    return resp.get("exit_code"), exec_output_text(resp) + exec_output_text(resp, "stderr")


def test_chown_in_the_workspace_succeeds(serial_env):
    client, name = serial_env
    code, out = _run(
        client,
        name,
        "set -e; cd /root; rm -rf own; echo x > own; chown 0:0 own; chown 1000:1000 own; "
        "chown -h 0 own; stat -c '%u:%g' own",
    )
    assert code == 0, out
    assert out.strip().splitlines()[-1] == "0:0", out


def test_cp_archive_into_the_workspace_keeps_modes(serial_env):
    client, name = serial_env
    code, out = _run(
        client,
        name,
        "set -e; rm -rf /tmp/src /root/dst; mkdir /tmp/src; echo y > /tmp/src/a; "
        "chmod 755 /tmp/src; chmod 640 /tmp/src/a; cp -a /tmp/src /root/dst; "
        "stat -c '%a' /root/dst /root/dst/a",
    )
    assert code == 0, out
    assert out.strip().splitlines()[-2:] == ["755", "640"], out
