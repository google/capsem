"""Native pipes and processes exercise the startup controller, without OCI mocks."""

import runpy
import struct
import sys

import msgpack
import pytest

from tests.fixtures.oci.test_launch_config import SOURCE


@pytest.fixture
def preparation():
    return runpy.run_path(str(SOURCE.with_name("runtime_preparation.py")))


def frame(value):
    payload = msgpack.packb(value, use_bin_type=True)
    return struct.pack(">I", len(payload)) + payload


def test_startup_configuration_preserves_the_original_process_and_argv(preparation):
    original = {"process": {"args": ["entry", "a b", "--help"], "user": {"uid": 1001},
                            "cwd": "/workspace", "env": ["KEY=value"], "noNewPrivileges": True},
                "mounts": [], "hooks": {"poststart": ["running"], "prestart": ["network"]}}
    startup = {"script": "printf '%s' \'approved\'", "timeout_seconds": 121}
    configured = preparation["configure"](original, startup)
    assert configured["process"]["args"] == ["/run/capsem-prepare", "121", startup["script"], "--", "entry", "a b", "--help"]
    assert configured["process"] | {"args": None} == {"args": None, "user": {"uid": 1001},
        "cwd": "/workspace", "env": ["KEY=value"], "noNewPrivileges": True}
    assert configured["hooks"] == {"prestart": ["network"]}
    assert configured["mounts"] == [{"destination": "/run/capsem-prepare", "source": "/run/capsem-sysutil",
                                     "type": "bind", "options": ["bind", "ro", "nosuid", "nodev"]}]
    assert preparation["configure"](original, None) is original
    for invalid in ({}, {**startup, "timeout_seconds": True}, {**startup, "timeout_seconds": 0},
                    {**startup, "script": "x" * 65537}, {**startup, "script": "bad\0script"}):
        with pytest.raises(ValueError):
            preparation["checked"](invalid)


def test_private_status_waits_for_exec_eof_and_output_uses_a_separate_pipe(preparation, tmp_path):
    startup = {"script": ":", "timeout_seconds": 5}
    events = frame({"type": "preparing"}) + frame({"type": "entrypoint"})
    with preparation["Bridge"](tmp_path, startup, lambda: pytest.fail("unexpected kill")) as bridge:
        process = bridge.spawn([sys.executable, "-c",
            f"import os; os.write(4, {events!r}); os.write(3, b'startup output'); os.close(4); os.write(1, b'entrypoint output')"],
            stdout=-1, stderr=-1)
        assert not (tmp_path / "running").exists()
        assert bridge.wait(process) == 0
        with process.stdout as stdout, process.stderr as stderr:
            assert stdout.read() == b"entrypoint output"
            assert stderr.read() == b""
        assert bridge.tail == b"startup output"
        assert (tmp_path / "running").read_bytes() == b"1\n"
        assert not (tmp_path / "preparing").exists()


@pytest.mark.parametrize(("failure", "expected"), [
    ({"kind": "exit", "code": 17}, 17), ({"kind": "signal", "signal": 15}, 143),
    ({"kind": "timed_out"}, 124), ({"kind": "shell_unavailable"}, 126),
    ({"kind": "entrypoint_exec"}, 126), ({"kind": "spawn"}, 126),
    ({"kind": "wait"}, 126), ({"kind": "invalid_input"}, 126),
])
def test_private_preparation_failure_is_authoritative_over_runner_success(preparation, tmp_path, failure, expected):
    events = frame({"type": "preparing"}) + frame({"type": "failed", "failure": failure})
    with preparation["Bridge"](tmp_path, {"script": ":", "timeout_seconds": 5}, lambda: None) as bridge:
        process = bridge.spawn([sys.executable, "-c", f"import os; os.write(4, {events!r})"])
        assert bridge.wait(process) == expected
        assert process.returncode == 0, "the private status, not runner success, determines preparation failure"
        assert not (tmp_path / "running").exists()
        assert not (tmp_path / "preparing").exists()


def test_external_deadline_stops_a_wrapper_that_cannot_run_its_own_timer(preparation, tmp_path):
    startup = {"script": ":", "timeout_seconds": 1}
    events = frame({"type": "preparing"})
    processes = []
    with preparation["Bridge"](tmp_path, startup, lambda: processes[0].kill()) as bridge:
        process = bridge.spawn([sys.executable, "-c",
            f"import os,signal; os.write(4, {events!r}); os.kill(os.getpid(), signal.SIGSTOP)"])
        processes.append(process)
        assert bridge.wait(process) == 124
        assert process.returncode is not None and process.returncode < 0, "external deadline must kill and reap the stopped child"
        assert (tmp_path / "failed").read_text().startswith("startup timed_out\n")
        assert not (tmp_path / "running").exists()


def test_failure_tail_is_bounded_sanitized_and_cannot_follow_a_marker_symlink(preparation, tmp_path, capfdbinary):
    target = tmp_path / "preserve-user-file"
    target.write_text("preserved")
    (tmp_path / "failed").symlink_to(target)
    events = frame({"type": "preparing"}) + frame({"type": "failed", "failure": {"kind": "exit", "code": 17}})
    with preparation["Bridge"](tmp_path, {"script": ":", "timeout_seconds": 5}, lambda: None) as bridge:
        process = bridge.spawn([sys.executable, "-c",
            f"import os; os.write(3, b'\\x1b[31m' + b'x'*70000); os.write(4, {events!r})"])
        assert bridge.wait(process) == 17, "a private preparation failure cannot become a successful runner exit"
        assert process.returncode == 0, "the fixture deliberately contradicts the private failure with runner success"
        failed = (tmp_path / "failed").read_bytes()
        captured = capfdbinary.readouterr()
        assert captured.out == b""
        assert captured.err == failed + b"\n", "attached callers need the diagnostic before teardown"
        assert failed.startswith(b"startup exit (17)\n")
        assert failed.endswith(b"[startup output truncated]")
        assert len(failed) < 33000
        assert b"\x1b" not in failed
        assert target.read_text() == "preserved"
        assert not (tmp_path / "running").exists()


@pytest.mark.parametrize("payload", [b"\x81\xa4type\xa9preparing\x00", b"\xc0", b"\x81\xa4type\x80",
    msgpack.packb({"type": "failed", "failure": {"kind": "exit", "code": 0}}),
    msgpack.packb({"type": "preparing", "extra": "refuse"})])
def test_invalid_status_is_refused(preparation, payload):
    with pytest.raises((ValueError, msgpack.UnpackException)):
        preparation["status_record"](payload)


def test_output_backpressure_disarms_the_pipe_until_console_capacity_returns(preparation, tmp_path):
    import selectors

    with preparation["Bridge"](tmp_path, {"script": ":", "timeout_seconds": 5}, lambda: None) as bridge, selectors.DefaultSelector() as selector:
        bridge.can_output = lambda: False
        bridge.watch(selector)
        assert bridge.output not in selector.get_map()
        assert bridge.status in selector.get_map()
        # A stopped console cannot make a readable pipe spin the actor.
        import os
        os.write(bridge.output_write, b"accepted once capacity returns")
        bridge.output_ready()
        assert not bridge.tail
        bridge.can_output = lambda: True
        bridge.watch(selector)
        assert bridge.output in selector.get_map()
        bridge.output_ready()
        assert bridge.tail == b"accepted once capacity returns"

def test_spawn_failure_closes_partial_duplicate_and_writer_ownership(preparation, tmp_path, monkeypatch):
    import errno
    import os

    original = preparation["fcntl"].fcntl
    duplicated = []
    def refuse_second(fd, command, minimum):
        if duplicated:
            raise OSError(errno.EMFILE, "fixture descriptor pressure")
        actual = original(fd, command, minimum)
        duplicated.append(actual)
        return actual
    with preparation["Bridge"](tmp_path, {"script": ":", "timeout_seconds": 5}, lambda: None) as bridge:
        monkeypatch.setattr(preparation["fcntl"], "fcntl", refuse_second)
        with pytest.raises(OSError, match="descriptor pressure"):
            bridge.spawn(["never-executed"])
        assert bridge.output_write is None and bridge.status_write is None
        with pytest.raises(OSError) as closed:
            os.fstat(duplicated[0])
        assert closed.value.errno == errno.EBADF
