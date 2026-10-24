"""Strict measured TCP to the disposable installed owner's private gateway."""

from __future__ import annotations

import os
import re
import stat
from dataclasses import dataclass, field
from pathlib import Path

from helpers.http_transport import Transport


@dataclass(frozen=True)
class _Markers:
    port: int
    gateway_pid: int
    service_pid: int
    token: str = field(repr=False)
    inodes: tuple[tuple[int, int], ...]


def _markers(root: Path, maximum: int) -> _Markers:
    if not root.is_absolute() or type(maximum) is not int or maximum <= 0:
        raise ValueError(
            "installed gateway requires an absolute owned runtime and finite byte budget"
        )
    directory = None
    try:
        directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        owner = os.fstat(directory)
        if owner.st_uid != os.getuid() or owner.st_mode & 0o022:
            raise ValueError("installed gateway runtime has foreign ownership or permissions")
        values, inodes = {}, []
        for name in ("gateway.token", "gateway.port", "gateway.pid", "service.pid"):
            descriptor = os.open(
                name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=directory
            )
            try:
                before = os.fstat(descriptor)
                # Gateway's public token format is64 alphanumeric bytes;
                # decimal PID/port markers are smaller. Do not allocate the
                # much larger response budget for a hostile marker file.
                limit = min(maximum, 65)
                if (
                    not stat.S_ISREG(before.st_mode)
                    or before.st_uid != owner.st_uid
                    or before.st_mode & 0o022
                    or (name == "gateway.token" and before.st_mode & 0o077)
                    or not 0 < before.st_size <= limit
                ):
                    raise ValueError("installed gateway marker is not a private regular owned file")
                raw = os.read(descriptor, limit + 1)
                after = os.fstat(descriptor)
                if (
                    len(raw) != before.st_size
                    or len(raw) > limit
                    or (before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                    != (after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                ):
                    raise ValueError("installed gateway marker changed while reading")
                values[name] = raw.decode("ascii").strip()
                inodes.append((before.st_dev, before.st_ino))
            finally:
                os.close(descriptor)
        token = values["gateway.token"]
        if re.fullmatch(r"[A-Za-z0-9]{64}", token) is None:
            raise ValueError("installed gateway token marker has invalid format")
        numbers = []
        for name in ("gateway.port", "gateway.pid", "service.pid"):
            value = values[name]
            if not value.isascii() or not value.isdecimal() or len(value) > 19 or int(value) <= 0:
                raise ValueError("installed gateway numeric marker has invalid format")
            numbers.append(int(value))
        if numbers[0] > 65535:
            raise ValueError("installed gateway port marker is outside the TCP range")
        if any(value > (1 << 32) - 1 for value in numbers[1:]):
            raise ValueError("installed gateway PID marker is outside the process ID range")
        return _Markers(numbers[0], numbers[1], numbers[2], token, tuple(inodes))
    except (OSError, UnicodeError):
        raise ValueError("installed gateway runtime markers are unavailable") from None
    finally:
        if directory is not None:
            os.close(directory)


class GatewayTransport:
    """Header/identity adaptation only; Transport still owns every HTTP byte."""

    def __init__(self, runtime: Path, *, maximum: int):
        self._runtime = runtime
        self._maximum = maximum
        self._identity = _markers(runtime, maximum)
        self._transport = Transport(host="127.0.0.1", port=self._identity.port)

    def measure_request(self, method, path, *, headers=None, **kwargs):
        if _markers(self._runtime, self._maximum) != self._identity:
            raise ValueError(
                "installed gateway generation changed; original request must not be replayed"
            )
        return self._transport.measure_request(
            method,
            path,
            headers={**(headers or {}), "Authorization": f"Bearer {self._identity.token}"},
            **kwargs,
        )

    def close(self):
        self._transport.close()
