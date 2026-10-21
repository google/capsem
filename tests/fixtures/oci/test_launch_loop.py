"""Published filesystems keep the verified descriptor through loop setup."""

import contextlib
import errno
import json
import os
import struct
from pathlib import Path

import pytest

from tests.fixtures.oci.test_launch_config import (
    SECURITY,
    _blob,
    _share,
    crypto,
    launcher,
    unpacked,
)

__all__ = ["launcher"]


def test_published_root_mounts_the_loop_bound_to_its_verified_descriptor(
    launcher, tmp_path, monkeypatch
):
    monkeypatch.setattr(launcher, "RUNTIME", tmp_path / "runtime")
    launcher.RUNTIME.mkdir()
    share, manifest, _ = _share(tmp_path)
    data = b"verified published root"
    digest = _blob(share, data)
    pinned = []

    @contextlib.contextmanager
    def loop_device(fd):
        assert os.pread(fd, len(data), 0) == data
        path = share / digest.removeprefix("sha256:")
        path.unlink()
        path.write_bytes(b"replacement must never be reopened")
        assert os.pread(fd, len(data), 0) == data
        pinned.append(fd)
        yield "/dev/loop7"
        assert os.pread(fd, len(data), 0) == data

    monkeypatch.setattr(launcher, "loop_device", loop_device, raising=False)
    calls = []

    def command(*args, **kwargs):
        calls.append(args)
        if args[0] == "mount":
            assert args == (
                "mount", "-t", "erofs", "-o", "ro,nosuid,nodev", "/dev/loop7",
                str(launcher.RUNTIME / "bundle/rootfs"),
            )
            assert not kwargs
        elif args[:3] == ("umoci", "raw", "runtime-config"):
            Path(args[-1]).write_text(json.dumps(unpacked()))

    descriptor = {
        "digest": digest,
        "integrity": "blake3:" + crypto.blake3(data).hexdigest(),
        "size": len(data),
    }
    with launcher.published_root(
        {"manifest": manifest, "rootfs": descriptor}, SECURITY["id_map"],
        share=lambda: contextlib.nullcontext(share), run=command,
    ):
        assert len(pinned) == 1
    assert calls[-1] == ("umount", str(launcher.RUNTIME / "bundle/rootfs"))


@pytest.mark.parametrize("failure", [None, "control", "select", "device", "configure", "body"])
def test_loop_configuration_pins_the_read_only_blob_and_closes_every_handle(
    launcher, tmp_path, monkeypatch, failure
):
    payload = tmp_path / "payload"
    payload.write_bytes(b"verified root")
    opened, calls = [], []
    original_open = os.open

    def fail(stage):
        if failure == stage:
            raise OSError(errno.EIO, stage)

    with payload.open("rb") as blob:
        def open_device(path, flags, *args, **kwargs):
            if path not in ("/dev/loop-control", "/dev/loop7"):
                return original_open(path, flags, *args, **kwargs)
            stage = "control" if path.endswith("-control") else "device"
            fail(stage)
            if stage == "device":
                # Check before opening: the kernel may reuse the closed fd number.
                with pytest.raises(OSError):
                    os.fstat(opened[0])
            access = os.O_RDWR if stage == "control" else os.O_RDONLY
            assert flags == access | os.O_CLOEXEC | os.O_NOFOLLOW
            fd = original_open(payload, os.O_RDONLY)
            opened.append(fd)
            return fd

        def ioctl(fd, request, *args):
            calls.append(request)
            if request == 0x4C82:
                fail("select")
                return 7
            assert request == 0x4C0A
            fail("configure")
            config, = args
            # Linux UAPI loop_config: fd, block size, loop_info64, reserved.
            assert len(config) == 304
            assert struct.unpack_from("=I", config, 0)[0] == blob.fileno()
            assert struct.unpack_from("=I", config, 60)[0] == 5  # read-only + autoclear
            assert config[4:60] == bytes(56) and config[64:] == bytes(240)
            assert os.pread(blob.fileno(), 13, 0) == b"verified root"
            return 0

        monkeypatch.setattr(launcher.os, "open", open_device)
        monkeypatch.setattr(launcher.fcntl, "ioctl", ioctl)
        expected = contextlib.nullcontext() if failure is None else pytest.raises(OSError, match=failure)
        with expected, launcher.loop_device(blob.fileno()) as device:
            assert device == "/dev/loop7"
            assert os.fstat(opened[1])
            fail("body")
        assert os.pread(blob.fileno(), 13, 0) == b"verified root"
    for fd in opened:
        with pytest.raises(OSError):
            os.fstat(fd)
    if failure is None:
        assert calls == [0x4C82, 0x4C0A]
