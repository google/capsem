"""Run one bounded import with the workload's existing user-namespace map.

Guest-only launcher code. A mapped VM uid alone is not equivalent to the
workload's caller-relative identity on Apple's VirtioFS server.
"""

import ctypes
import os
from pathlib import Path

CLONE_NEWUSER = 0x10000000


def identity(id_map, owner):
    if (
        not isinstance(id_map, dict)
        or set(id_map) != {"containerID", "hostID", "size"}
        or any(type(value) is not int for value in id_map.values())
        or id_map["containerID"] != 0
        or id_map["hostID"] < 65536
        or not 0 < id_map["size"] <= 65536
    ):
        raise ValueError("invalid scratch import user map")
    user = tuple(value - id_map["hostID"] for value in owner)
    if len(user) != 2 or any(not 0 <= value < id_map["size"] for value in user):
        raise ValueError("scratch import user is outside workload map")
    return user, f"0 {id_map['hostID']} {id_map['size']}\n"


def run_as_user(id_map, owner, operation):
    user, mapping = identity(id_map, owner)
    if os.geteuid() != 0:
        raise ValueError("scratch import namespace requires the guest launcher")
    ready_read, ready_write = os.pipe()
    resume_read, resume_write = os.pipe()
    child = os.fork()
    if child == 0:
        status = 1
        try:
            os.close(ready_read)
            os.close(resume_write)
            os.setgroups([])
            if ctypes.CDLL(None, use_errno=True).unshare(CLONE_NEWUSER) != 0:
                raise OSError("cannot create scratch import user namespace")
            os.write(ready_write, b"1")
            os.close(ready_write)
            if os.read(resume_read, 1) != b"1":
                raise OSError("scratch import user map was not installed")
            os.close(resume_read)
            # unshare starts with an unmapped caller uid. Transition through
            # namespace root so selecting a nonroot image uid drops its
            # namespace capabilities through the normal setuid semantics.
            os.setuid(0)
            os.setgid(user[1])
            os.setuid(user[0])
            operation()
            status = 0
        finally:
            os._exit(status)
    os.close(ready_write)
    os.close(resume_read)
    try:
        if os.read(ready_read, 1) != b"1":
            raise ValueError("scratch import user namespace was not created")
        for name in ("uid_map", "gid_map"):
            Path(f"/proc/{child}/{name}").write_text(mapping)
        os.write(resume_write, b"1")
    finally:
        os.close(ready_read)
        os.close(resume_write)
        _, status = os.waitpid(child, 0)
    if status != 0:
        raise ValueError("scratch import failed as the image user")
