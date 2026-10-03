"""The workspace can be idmapped for a workload in a user namespace.

A workload maps container uids 0-65535 to VM uids 100000-165535. The
workspace's VirtioFS server reports every entry as 0:0, so without an idmapped
mount container root would see /root owned by nobody and fail to chown,
`tar -x` or `cp -a` in it. Apple's server never offers FUSE_ALLOW_IDMAP, and
the guest kernel used to refuse the idmap with EINVAL
(config/docker/image/kernel/patches/0002-*).

The probe runs where the container launcher runs -- a private mount namespace
with the root moved onto / -- because the agent's exec shell is chrooted and
the kernel refuses it a user namespace.
"""

import base64

import pytest
from helpers.service import exec_output_text

pytestmark = pytest.mark.serial

IDMAP_PROBE = r"""
import ctypes, errno, os, subprocess, time

libc = ctypes.CDLL(None, use_errno=True)
OPEN_TREE, MOVE_MOUNT, MOUNT_SETATTR = 428, 429, 442
OPEN_TREE_CLONE, AT_EMPTY_PATH = 1, 0x1000
MOUNT_ATTR_IDMAP, MOVE_MOUNT_F_EMPTY_PATH = 0x00100000, 4


class Attr(ctypes.Structure):
    _fields_ = [("set", ctypes.c_uint64), ("clr", ctypes.c_uint64),
                ("prop", ctypes.c_uint64), ("userns_fd", ctypes.c_uint64)]


holder = subprocess.Popen(["unshare", "-U", "sleep", "60"])
time.sleep(0.3)
for name in ("uid_map", "gid_map"):
    with open(f"/proc/{holder.pid}/{name}", "w") as f:
        f.write("0 100000 65536")
userns = os.open(f"/proc/{holder.pid}/ns/user", os.O_RDONLY)
os.makedirs("/root/idmap-probe", exist_ok=True)
with open("/root/idmap-probe/file", "w") as f:
    f.write("x")
tree = libc.syscall(OPEN_TREE, -100, b"/root", OPEN_TREE_CLONE)
attr = Attr(MOUNT_ATTR_IDMAP, 0, 0, userns)
if libc.syscall(MOUNT_SETATTR, tree, b"", AT_EMPTY_PATH, ctypes.byref(attr), ctypes.sizeof(attr)) < 0:
    print("IDMAP", errno.errorcode[ctypes.get_errno()])
else:
    os.makedirs("/tmp/idmapped", exist_ok=True)
    libc.syscall(MOVE_MOUNT, tree, b"", -100, b"/tmp/idmapped", MOVE_MOUNT_F_EMPTY_PATH)
    print("IDMAP ok")
    print("IDMAPPED", os.stat("/tmp/idmapped/idmap-probe/file").st_uid)
    os.chown("/tmp/idmapped/idmap-probe/file", 100000, 100000)
    print("CHOWN ok")
    subprocess.run(["umount", "/tmp/idmapped"], check=True)
print("PLAIN", os.stat("/root/idmap-probe/file").st_uid)
holder.kill()
"""


def test_the_workspace_can_be_idmapped_for_a_user_namespace(serial_env):
    client, name = serial_env
    probe = base64.b64encode(IDMAP_PROBE.encode()).decode()
    command = (
        f"echo {probe} | base64 -d > /tmp/idmap_probe.py; "
        "nsenter -t 1 -m -r /bin/busybox unshare -m /bin/sh -ec "
        "'mount --make-rprivate /; cd /newroot; mount --move . /; "
        "exec chroot . /usr/bin/python3 /tmp/idmap_probe.py' 2>&1"
    )
    resp = client.post(f"/vms/{name}/exec", {"command": command, "timeout_secs": 60})
    assert resp is not None
    out = exec_output_text(resp) + exec_output_text(resp, "stderr")
    lines = out.split()
    assert "IDMAP ok" in out, out
    # VM uid 0 shows as 100000 -- container root -- through the idmapped mount.
    assert lines[lines.index("IDMAPPED") + 1] == "100000", out
    assert "CHOWN ok" in out, out
    # The share's own mount keeps the identity mapping.
    assert lines[lines.index("PLAIN") + 1] == "0", out
