"""Session terminal metadata remains observable without any terminal viewer."""

import shlex
import time

import msgpack
import pytest
from helpers.image_session import image_session, workload_exec

from tests.ironbank.kingslanding.test_run import service
from tests.ironbank.kingslanding.test_workload_terminal import (
    probe,
    terminal,
)

__all__ = ["service"]
pytestmark = pytest.mark.integration


def terminal_summary(client, vm_id, title, *, notification=None, bell_count=None, timeout=5):
    end = time.monotonic() + timeout
    summary = None
    while time.monotonic() < end:
        row = next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == vm_id)
        summary = row.get("terminal")
        if summary and summary["title"] == title:
            notice = summary.get("last_notification")
            if (bell_count is None or summary["bell_count"] == bell_count) and (
                notification is None or (
                    notice is not None and (notice["title"], notice["body"]) == notification
                )
            ):
                return summary
        time.sleep(0.05)
    raise AssertionError(
        f"/vms/list did not report terminal title {title!r}, "
        f"notification {notification!r}, bell count {bell_count!r}: {summary!r}"
    )


def test_terminal_title_bell_and_notification_are_listed_without_a_viewer(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-state") as vm_id:
        result = workload_exec(
            client,
            vm_id,
            r"printf '\033]2;Capsem verification\007\007\033]777;notify;Review;Ready\007' > /proc/1/fd/1",
        )
        assert result["exit_code"] == 0, result
        summary = terminal_summary(
            client, vm_id, "Capsem verification", notification=("Review", "Ready"), bell_count=1,
        )
        assert summary["bell_count"] == 1, summary
        assert summary["last_notification"]["title"] == "Review", summary
        assert summary["last_notification"]["body"] == "Ready", summary
        assert summary["last_notification"]["id"], summary
        assert summary["revision"] > 0, summary
        # Repeated list reads are observations: no bells or alerts are created.
        for _ in range(3):
            assert terminal_summary(client, vm_id, "Capsem verification") == summary


def next_state(socket, predicate):
    end = time.monotonic() + 10
    while time.monotonic() < end:
        frame = socket.recv(timeout=max(0.01, end - time.monotonic()))
        if isinstance(frame, bytes) and "status" in (message := msgpack.unpackb(frame)):
            status = message["status"]
            assert status["type"] != "error", status
            if status["type"] == "terminal" and predicate(status["state"]):
                return status["state"]
    raise AssertionError("terminal did not publish the expected state")


def test_terminal_snapshot_keeps_colors_and_focus_controls_the_actual_pty(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-snapshot") as vm_id:
        result = workload_exec(client, vm_id, (
            r"(printf '\033[38;2;10;20;30m'; head -c 70000 /dev/zero | tr '\000' x; "
            r"printf '\033[2J\033[HRGB\033]2;Snapshot ready\007\007') > /proc/1/fd/1"
        ))
        assert result["exit_code"] == 0, result
        summary = terminal_summary(client, vm_id, "Snapshot ready", bell_count=1)
        with terminal(service, vm_id) as first:
            snapshot = next_state(first, lambda state: state["reset"] and state["title"] == "Snapshot ready")
            assert snapshot["events"] == [], "attaching cannot replay a bell"
            assert snapshot["bell_count"] == summary["bell_count"]
            seen = b""
            end = time.monotonic() + 5
            while b"RGB" not in seen and time.monotonic() < end:
                frame = first.recv(timeout=5)
                if isinstance(frame, bytes):
                    seen += msgpack.unpackb(frame).get("stdout", b"")
            assert b"RGB" in seen and b"38;2;10;20;30" in seen, seen[-2000:]
            first.send(msgpack.packb({"control": {"type": "focus", "cols": 101, "rows": 33}}, use_bin_type=True))
            next_state(first, lambda state: (state["cols"], state["rows"]) == (101, 33))
            with terminal(service, vm_id) as second:
                second.send(msgpack.packb({"control": {"type": "focus", "cols": 60, "rows": 20}}, use_bin_type=True))
                next_state(second, lambda state: (state["cols"], state["rows"]) == (60, 20))
                # A background view cannot overwrite the focused view's dimensions.
                first.send(msgpack.packb({"control": {"type": "resize", "cols": 150, "rows": 45}}, use_bin_type=True))
                end = time.monotonic() + 5
                size = None
                while time.monotonic() < end:
                    size = workload_exec(client, vm_id, "stty -F /proc/1/fd/0 size")
                    if size["exit_code"] == 0 and size["stdout_text"].strip() == "20 60":
                        break
                    time.sleep(0.05)
                assert size is not None
                assert size["exit_code"] == 0 and size["stdout_text"].strip() == "20 60", size
        # SIGWINCH may redraw bash's prompt and publish its own newer title.
        latest = next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == vm_id)["terminal"]
        assert latest["title"] and latest["revision"] >= summary["revision"], latest
        assert latest["bell_count"] == summary["bell_count"], latest


def test_workload_advertises_full_color_to_its_terminal_apps(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-colors") as vm_id:
        result = workload_exec(client, vm_id, 'printf "%s\\n%s\\n" "$TERM" "$COLORTERM"')
        assert result["exit_code"] == 0, result
        assert result["stdout_text"].splitlines() == ["xterm-256color", "truecolor"], result


def test_the_application_owns_cooked_and_raw_terminal_modes(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-line-discipline") as vm_id, terminal(service, vm_id) as socket:
        assert probe(socket) == ("1000", "100000", "workload")
        modes = workload_exec(client, vm_id, "stty -F /proc/1/fd/0 -a")
        assert modes["exit_code"] == 0, modes
        flags = {flag.rstrip(";") for flag in modes["stdout_text"].split()}
        assert {"opost", "onlcr"} <= flags, modes
        result = workload_exec(client, vm_id, (
            r"printf '\033[2J\033[Hfirst\nsecond\n\033]2;Cooked checkpoint\007' > /proc/1/fd/1"
        ))
        assert result["exit_code"] == 0, result
        state = next_state(socket, lambda state: state["title"] == "Cooked checkpoint")
        assert (state["cursor"]["row"], state["cursor"]["col"]) == (2, 0), state

        # This is a real foreground app on the image command's terminal.
        # Bare LF preserves its column in raw mode; escape keys arrive
        # immediately without a newline or an intervening terminal echo.
        script = r"""
import os, signal, termios, tty
before = termios.tcgetattr(0)
def resized(*_):
    size = os.get_terminal_size(0)
    os.write(1, f'\x1b]2;Raw size {size.lines} {size.columns}\x07'.encode())
try:
    tty.setraw(0)
    signal.signal(signal.SIGWINCH, resized)
    os.write(1, b'\x1b[2J\x1b[Habcdefgh\n\n\b\bX\x1b]2;Raw checkpoint\x07')
    if os.read(0, 3) == b'\x1b[A':
        os.write(1, b'\x1b]2;Raw key received\x07')
    os.read(0, 1)
finally:
    termios.tcsetattr(0, termios.TCSANOW, before)
"""
        command = "python3 -c " + shlex.quote(script) + "\n"
        socket.send(msgpack.packb({"stdin": command.encode()}, use_bin_type=True))
        state = next_state(socket, lambda state: state["title"] == "Raw checkpoint")
        assert (state["cursor"]["row"], state["cursor"]["col"]) == (2, 7), state
        socket.send(msgpack.packb({"stdin": b"\x1b[A"}, use_bin_type=True))
        state = next_state(socket, lambda state: state["title"] == "Raw key received")
        assert (state["cursor"]["row"], state["cursor"]["col"]) == (2, 7), state
        for rows, cols in ((33, 101), (20, 60), (45, 150)):
            socket.send(msgpack.packb({"control": {"type": "resize", "cols": cols, "rows": rows}}, use_bin_type=True))
            state = next_state(socket, lambda state, title=f"Raw size {rows} {cols}": state["title"] == title)
            assert (state["rows"], state["cols"]) == (rows, cols), state
        socket.send(msgpack.packb({"stdin": b"q"}, use_bin_type=True))


def test_unterminated_control_string_burst_recovers_without_replacing_title(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-control-budget") as vm_id:
        result = workload_exec(client, vm_id, r"printf '\033]2;Retained\007' > /proc/1/fd/1")
        assert result["exit_code"] == 0, result
        terminal_summary(client, vm_id, "Retained")
        result = workload_exec(client, vm_id, (
            r"(printf '\033]2;'; head -c 8388608 /dev/zero | tr '\000' x; "
            r"printf '\007\033]777;notify;Recovered;Ready\007') > /proc/1/fd/1"
        ))
        assert result["exit_code"] == 0, result
        # The retained title predates the burst. PTY drainage and notification
        # parsing can still be in flight after the workload RPC completes.
        summary = terminal_summary(
            client, vm_id, "Retained", notification=("Recovered", "Ready"), timeout=10,
        )
        assert summary["title"] == "Retained", summary
        assert summary["last_notification"]["title"] == "Recovered", summary
        assert summary["last_notification"]["body"] == "Ready", summary


def test_quiet_synchronized_output_expires_and_answers_its_query_once(service, tmp_path):
    client = service.client()
    with image_session(service, tmp_path, "terminal-quiet-sync") as vm_id, terminal(service, vm_id) as socket:
        assert probe(socket) == ("1000", "100000", "workload")
        baseline = next(row for row in client.get("/vms/list")["sandboxes"] if row["id"] == vm_id)["terminal"]
        # This foreground app emits no sync-end and no later PTY bytes. It
        # cannot receive the query reply until the host expires the update.
        script = r"""
import os, select, termios, tty
from pathlib import Path
before = termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1, b'\x1b[2J\x1b[H\x1b[?2026hquiet\x07\x1b]2;Quiet synchronized\x07\x1b]777;notify;Sync;Expired\x07\x1b[6n')
    reply = b''
    while not reply.endswith(b'R'):
        reply += os.read(0, 64)
    if select.select([0], [], [], 0.5)[0]:
        reply += os.read(0, 64)
    Path('/tmp/capsem-sync-query').write_bytes(reply)
    os.read(0, 1)
finally:
    termios.tcsetattr(0, termios.TCSANOW, before)
"""
        socket.send(msgpack.packb({"stdin": ("python3 -c " + shlex.quote(script) + "\n").encode()}, use_bin_type=True))
        state = next_state(socket, lambda state: state["title"] == "Quiet synchronized"
                           and state.get("last_notification") is not None
                           and state["last_notification"]["title"] == "Sync"
                           and state["last_notification"]["body"] == "Expired"
                           and state["bell_count"] == baseline["bell_count"] + 1)
        summary = terminal_summary(
            client, vm_id, "Quiet synchronized", notification=("Sync", "Expired"),
            bell_count=baseline["bell_count"] + 1,
        )
        assert state["bell_count"] == summary["bell_count"] == baseline["bell_count"] + 1
        assert summary["last_notification"]["title"] == "Sync", summary
        assert summary["last_notification"]["body"] == "Expired", summary
        result = workload_exec(client, vm_id, (
            "python3 -c " + shlex.quote(
                "from pathlib import Path; import time; p=Path('/tmp/capsem-sync-query'); "
                "end=time.monotonic()+5\n"
                "while not p.exists() and time.monotonic()<end: time.sleep(0.05)\n"
                "print(p.read_bytes().hex())"
            )
        ))
        assert result["exit_code"] == 0, result
        assert result["stdout_text"].strip() == b"\x1b[1;6R".hex(), result
        for _ in range(3):
            assert terminal_summary(client, vm_id, "Quiet synchronized") == summary
        socket.send(msgpack.packb({"stdin": b"q"}, use_bin_type=True))
