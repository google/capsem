"""Allowlisted identity receipts from the original synthetic warm foreground."""

import errno
import json
import os


def _read(path, limit):
    with open(path, "rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("identity metadata exceeds its bound")
    return data.decode("ascii")


def _capture(operation):
    try:
        return operation()
    except OSError as error:
        return {"errno": error.errno or errno.EIO}
    except (UnicodeError, ValueError):
        return {"invalid": 1}


def _stat(value):
    return [value.st_dev, value.st_ino]


def _mount_id(text):
    values = [
        line.split(":", 1)[1].strip() for line in text.splitlines() if line.startswith("mnt_id:")
    ]
    if len(values) != 1 or not values[0].isascii() or not values[0].isdigit():
        raise ValueError("missing or invalid mount identity")
    return int(values[0])


def _mounts(text):
    result = {"root": [], "workspace": []}
    roles = {"/": "root", "/workspace": "workspace"}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 10:
            raise ValueError("invalid mount identity")
        role = roles.get(fields[4])
        if role is None:
            continue
        separator = fields.index("-")
        if separator < 6 or len(fields) != separator + 4:
            raise ValueError("invalid selected mount shape")
        kind = fields[separator + 1]
        if kind not in {"overlay", "erofs", "virtiofs", "tmpfs", "ext4"}:
            raise ValueError("unexpected filesystem kind")
        device = fields[2].split(":")
        numbers = [fields[0], fields[1], *device]
        if len(device) != 2 or any(not value.isascii() or not value.isdigit() for value in numbers):
            raise ValueError("invalid numeric mount identity")
        result[role].append(
            {
                "id": int(numbers[0]),
                "parent": int(numbers[1]),
                "device": [int(value) for value in device],
                "kind": kind,
            }
        )
        if len(result[role]) > 16:
            raise ValueError("too many matching mounts")
    if any(not rows for rows in result.values()):
        raise ValueError("missing fixed mount")
    return result


def snapshot(fds):
    if set(fds) - {"file", "directory", "image"}:
        raise ValueError("unexpected descriptor role")
    result = {
        "cwd": _capture(lambda: _stat(os.stat("."))),
        "namespace": _capture(lambda: os.stat("/proc/self/ns/mnt").st_ino),
        "mounts": _capture(lambda: _mounts(_read("/proc/self/mountinfo", 262144))),
    }
    for role, fd in fds.items():
        result[role] = {
            "stat": _capture(lambda fd=fd: _stat(os.fstat(fd))),
            "mount": _capture(lambda fd=fd: _mount_id(_read(f"/proc/self/fdinfo/{fd}", 4096))),
        }
    return result


def _failed(value):
    if isinstance(value, dict):
        return bool(set(value) & {"errno", "invalid"}) or any(
            _failed(item) for item in value.values()
        )
    if isinstance(value, list):
        return any(_failed(item) for item in value)
    return False


def capture(receipt_fd, phase, fds, previous=None):
    if phase not in {"before", "after"}:
        raise ValueError("unexpected warm phase")
    identity = snapshot(fds)
    payload = (json.dumps({"phase": phase, "identity": identity}, sort_keys=True) + "\n").encode()
    assert len(payload) <= 16384, "warm identity receipt exceeds its bound"
    try:
        remaining = memoryview(payload)
        while remaining:
            written = os.write(receipt_fd, remaining)
            if not written:
                raise OSError(errno.EIO, "identity write made no progress")
            remaining = remaining[written:]
        os.fsync(receipt_fd)
    except OSError as error:
        raise AssertionError(
            ("warm identity write", phase, error.errno, previous, identity)
        ) from None
    assert not _failed(identity), ("warm identity metadata", phase, previous, identity)
    return identity
