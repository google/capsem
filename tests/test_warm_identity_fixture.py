"""Source proof for warm diagnostics, not Apple VZ restore acceptance."""

import ast
import errno
import json
import os
from pathlib import Path
from types import SimpleNamespace

import pytest
from ironbank.kingslanding import warm_identity

MOUNTS = """10 1 0:2 / / rw,secret-option - erofs secret-source rw
20 10 0:3 /private-root /workspace rw,private-option - virtiofs private-source rw
30 10 0:4 /secret-root /private-user-path rw - tmpfs secret-data rw
"""


@pytest.fixture
def guest_metadata(monkeypatch):
    real_stat = os.stat
    monkeypatch.setattr(
        warm_identity,
        "_read",
        lambda path, _limit: (
            MOUNTS
            if path.endswith("mountinfo")
            else "pos: 99\nmnt_id:\t20\nsecret-field: private-value\n"
        ),
    )
    monkeypatch.setattr(
        warm_identity.os,
        "stat",
        lambda path, *args, **kwargs: (
            SimpleNamespace(st_ino=1234)
            if path == "/proc/self/ns/mnt"
            else real_stat(path, *args, **kwargs)
        ),
    )


def test_receipt_keeps_original_descriptor_identity(tmp_path, guest_metadata):
    path = tmp_path / "synthetic-file"
    path.write_bytes(b"private fixture content must never be reported")
    with path.open("rb") as stream:
        original = os.fstat(stream.fileno())
        receipt = warm_identity.snapshot({"file": stream.fileno()})
    assert receipt["file"]["stat"] == [original.st_dev, original.st_ino]
    assert receipt["file"]["mount"] == 20
    assert receipt["namespace"] == 1234
    serialized = json.dumps(receipt)
    for secret in ("private", "secret", "fixture content", str(path)):
        assert secret not in serialized
    assert receipt["mounts"] == {
        "root": [{"id": 10, "parent": 1, "device": [0, 2], "kind": "erofs"}],
        "workspace": [{"id": 20, "parent": 10, "device": [0, 3], "kind": "virtiofs"}],
    }


@pytest.mark.parametrize(
    "text", ["", "mnt_id: -2", "mnt_id: 2\nmnt_id: 3", "mnt_id: secret", "mnt_id: \uff11\uff12"]
)
def test_invalid_fd_metadata_is_not_accepted(text):
    with pytest.raises(ValueError):
        warm_identity._mount_id(text)


@pytest.mark.parametrize(
    "text",
    [
        "",
        "secret malformed row",
        MOUNTS.replace("0:3", "private"),
        MOUNTS.replace("virtiofs", "private-fs"),
        MOUNTS.splitlines()[0],
        MOUNTS + (MOUNTS.splitlines()[1] + "\n") * 17,
        MOUNTS.replace("- virtiofs private-source rw", "-"),
    ],
)
def test_invalid_or_unbounded_mount_metadata_is_not_accepted(text):
    with pytest.raises(ValueError):
        warm_identity._mounts(text)


def test_proc_read_is_bounded_and_does_not_emit_its_input(tmp_path):
    path = tmp_path / "synthetic-metadata"
    path.write_bytes(b"private" * 100)
    with pytest.raises(ValueError, match="exceeds its bound") as caught:
        warm_identity._read(path, 32)
    assert "private" not in str(caught.value)


def test_same_receipt_fd_persists_both_phases_and_reports_real_fd_failure(tmp_path, guest_metadata):
    path = tmp_path / "identity.log"
    with path.open("wb") as stream, (tmp_path / "held").open("w+b") as held:
        previous = warm_identity.capture(stream.fileno(), "before", {"file": held.fileno()})
        failed = os.dup(held.fileno())
        os.close(failed)
        with pytest.raises(AssertionError, match="warm identity metadata"):
            warm_identity.capture(stream.fileno(), "after", {"file": failed}, previous)
    records = [json.loads(line) for line in path.read_text().splitlines()]
    assert [row["phase"] for row in records] == ["before", "after"]
    assert records[1]["identity"]["file"]["stat"] == {"errno": errno.EBADF}


def test_failed_receipt_write_retains_sanitized_before_and_after(tmp_path, guest_metadata):
    path = tmp_path / "secret-path"
    path.write_bytes(b"secret fixture bytes")
    with path.open("rb") as stream:
        before = warm_identity.snapshot({"file": stream.fileno()})
        with pytest.raises(AssertionError, match="warm identity write") as caught:
            warm_identity.capture(stream.fileno(), "after", {"file": stream.fileno()}, before)
    assert caught.value.args[0][2] == errno.EBADF
    assert caught.value.args[0][3] == before
    assert caught.value.args[0][4] == before
    assert "secret" not in str(caught.value)


def test_unknown_roles_and_phases_fail_without_reporting_supplied_values(tmp_path):
    with pytest.raises(ValueError, match="descriptor role") as caught:
        warm_identity.snapshot({"private-path": 0})
    assert "private-path" not in str(caught.value)
    with pytest.raises(ValueError, match="warm phase"):
        warm_identity.capture(-1, "private-phase", {})


def test_short_receipt_writes_complete_same_record_without_reopening(
    tmp_path, guest_metadata, monkeypatch
):
    real_write = os.write
    with (tmp_path / "identity.log").open("w+b") as stream:
        with monkeypatch.context() as patch:
            patch.setattr(warm_identity.os, "write", lambda fd, data: real_write(fd, data[:7]))
            warm_identity.capture(stream.fileno(), "before", {})
        stream.seek(0)
        assert json.loads(stream.read())["phase"] == "before"


def test_injected_original_foreground_remains_valid_python():
    source = Path("tests/ironbank/kingslanding/test_terminal_suspend.py").read_text()
    scripts = [
        node.value
        for node in ast.walk(ast.parse(source))
        if isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and "original_identity = capture(" in node.value
    ]
    assert len(scripts) == 1
    compile(Path(warm_identity.__file__).read_text() + scripts[0], "warm-foreground", "exec")
