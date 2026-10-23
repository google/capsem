"""Guest scratch copy must use namespace-relative identity, not only VM uid."""

import ctypes
import os
import runpy
from pathlib import Path
from types import SimpleNamespace

import pytest

SOURCE = Path(__file__).resolve().parents[3] / "guest/artifacts/container/runtime_userns.py"
ID_MAP = {"containerID": 0, "hostID": 100000, "size": 65536}


def test_import_identity_uses_the_existing_workload_map():
    identity = runpy.run_path(str(SOURCE))["identity"]
    assert identity(ID_MAP, (101000, 101000)) == ((1000, 1000), "0 100000 65536\n")
    for mapping, owner in [
        ({**ID_MAP, "containerID": 1}, (101000, 101000)),
        ({**ID_MAP, "hostID": 0}, (1000, 1000)),
        (ID_MAP, (99999, 101000)),
        (ID_MAP, (165536, 101000)),
    ]:
        with pytest.raises(ValueError):
            identity(mapping, owner)


def test_parent_installs_both_maps_before_releasing_import_child(monkeypatch):
    run = runpy.run_path(str(SOURCE))["run_as_user"]
    events = []
    pipes = iter([(10, 11), (12, 13)])
    monkeypatch.setattr(os, "geteuid", lambda: 0)
    monkeypatch.setattr(os, "pipe", lambda: next(pipes))
    monkeypatch.setattr(os, "fork", lambda: 12345)
    monkeypatch.setattr(os, "close", lambda fd: None)
    monkeypatch.setattr(os, "read", lambda fd, count: b"1")
    monkeypatch.setattr(Path, "write_text", lambda path, text: events.append((str(path), text)))
    monkeypatch.setattr(os, "write", lambda fd, value: events.append((fd, value)))
    monkeypatch.setattr(os, "waitpid", lambda pid, flags: (pid, 0))
    run(ID_MAP, (101000, 101000), lambda: pytest.fail("parent cannot copy"))
    assert events == [
        ("/proc/12345/uid_map", "0 100000 65536\n"),
        ("/proc/12345/gid_map", "0 100000 65536\n"),
        (13, b"1"),
    ]


def test_child_enters_user_namespace_before_using_image_relative_ids(monkeypatch):
    run = runpy.run_path(str(SOURCE))["run_as_user"]
    events = []
    pipes = iter([(10, 11), (12, 13)])
    monkeypatch.setattr(os, "geteuid", lambda: 0)
    monkeypatch.setattr(os, "pipe", lambda: next(pipes))
    monkeypatch.setattr(os, "fork", lambda: 0)
    monkeypatch.setattr(os, "close", lambda fd: None)
    monkeypatch.setattr(os, "setgroups", lambda groups: events.append(("groups", groups)))
    monkeypatch.setattr(ctypes, "CDLL", lambda *args, **kwargs: SimpleNamespace(
        unshare=lambda flag: events.append(("unshare", flag)) or 0,
    ))
    monkeypatch.setattr(os, "write", lambda fd, value: events.append(("ready", value)))
    monkeypatch.setattr(os, "read", lambda fd, count: events.append(("mapped", fd)) or b"1")
    monkeypatch.setattr(os, "setgid", lambda gid: events.append(("gid", gid)))
    monkeypatch.setattr(os, "setuid", lambda uid: events.append(("uid", uid)))
    monkeypatch.setattr(os, "_exit", lambda status: (_ for _ in ()).throw(SystemExit(status)))
    with pytest.raises(SystemExit) as exited:
        run(ID_MAP, (101000, 101000), lambda: events.append(("copy",)))
    assert exited.value.code == 0
    assert events == [
        ("groups", []), ("unshare", 0x10000000), ("ready", b"1"),
        ("mapped", 12), ("gid", 1000), ("uid", 1000), ("copy",),
    ]
