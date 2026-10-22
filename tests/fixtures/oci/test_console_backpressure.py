"""A held workload PTY preserves duplex progress and exact byte order."""

import contextlib
import fcntl
import hashlib
import os
import select
import socket
import struct
import termios
import threading
import time

from tests.fixtures.oci.test_launch_config import launcher
from tests.fixtures.oci.test_launch_console import _read_until, held

__all__ = ["held", "launcher"]


def test_blocked_pty_input_keeps_output_resize_and_exact_pending_suffix(launcher, held):
    console, slave, path, _, _ = held
    # A real nonblocking PTY forces partial writes without replacing os.write.
    os.set_blocking(console._master, False)
    os.set_blocking(slave, False)
    payload = hashlib.shake_256(b"held-console-backpressure").digest(128 * 1024)
    with socket.socket(socket.AF_UNIX) as client:
        client.connect(str(path))
        client.sendall(launcher.encode_input(payload))
        assert select.select([slave], [], [], 2)[0], "no real PTY input was delivered"
        os.write(slave, b"OUTPUT-WHILE-INPUT-BLOCKED")
        assert b"OUTPUT-WHILE-INPUT-BLOCKED" in _read_until(client, b"OUTPUT-WHILE-INPUT-BLOCKED", timeout=2)
        client.sendall(launcher.encode_resize(33, 101))
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(slave, termios.TIOCGWINSZ, b"\0" * 8))
            if (rows, cols) == (33, 101):
                break
            time.sleep(.01)
        assert (rows, cols) == (33, 101)
        received = bytearray()
        deadline = time.monotonic() + 2
        while len(received) < len(payload) and time.monotonic() < deadline:
            if select.select([slave], [], [], max(0, deadline - time.monotonic()))[0]:
                received.extend(os.read(slave, len(payload) - len(received)))
        assert hashlib.sha256(received).digest() == hashlib.sha256(payload).digest(), (
            f"held console delivered {len(received)} of {len(payload)} input bytes"
        )


def test_attach_output_survives_a_blocked_input_socket_and_preserves_all_bytes(launcher):
    near, far = socket.socketpair()
    near.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
    keys_read, keys_write = os.pipe()
    screen_read, screen_write = os.pipe()
    payload = hashlib.shake_256(b"attach-relay-pressure").digest(256 * 1024)
    errors = []

    def run():
        try:
            launcher.relay(near, keys_read, screen_write)
        except OSError as error:
            errors.append(error)

    def produce():
        offset = 0
        try:
            while offset < len(payload):
                offset += os.write(keys_write, payload[offset:])
        except OSError:
            pass

    relay = threading.Thread(target=run, daemon=True)
    producer = threading.Thread(target=produce, daemon=True)
    relay.start()
    producer.start()
    try:
        assert select.select([far], [], [], 2)[0], "no real socket input arrived"
        far.sendall(launcher.encode_output(b"DUPLEX-OUTPUT"))
        assert select.select([screen_read], [], [], 2)[0], "blocked input stalled independent output"
        assert os.read(screen_read, 128) == b"DUPLEX-OUTPUT"
        received, reader = bytearray(), launcher.FrameReader()
        far.settimeout(2)
        while len(received) < len(payload):
            for kind, value in reader.feed(far.recv(65536)):
                if kind == "input":
                    received.extend(value)
        assert hashlib.sha256(received).digest() == hashlib.sha256(payload).digest()
    finally:
        far.close()
        relay.join(timeout=2)
        for fd in (keys_read, keys_write, screen_read, screen_write):
            with contextlib.suppress(OSError):
                os.close(fd)
        producer.join(timeout=2)
        near.close()
    assert not relay.is_alive() and not producer.is_alive()
    assert not errors, errors


def test_idle_attach_resize_wakes_the_actor_and_restores_caller_fd_flags(launcher):
    near, far = socket.socketpair()
    keys_read, keys_write = os.pipe()
    screen_read, screen_write = os.pipe()
    transport = launcher.Relay(near, keys_read, screen_write)
    worker = threading.Thread(target=transport.run, daemon=True)
    worker.start()
    try:
        transport.resize(33, 101)
        reader, events = launcher.FrameReader(), []
        far.settimeout(2)
        while not events:
            events.extend(reader.feed(far.recv(4096)))
        assert events == [("resize", (33, 101))]
    finally:
        far.close()
        worker.join(timeout=2)
        transport.close()
        assert not worker.is_alive()
        assert os.get_blocking(keys_read) and os.get_blocking(screen_write)
        assert near.gettimeout() is None
        near.close()
        for fd in (keys_read, keys_write, screen_read, screen_write):
            os.close(fd)


def test_blocked_display_backpressures_output_without_stalling_keys(launcher):
    near, far = socket.socketpair()
    far.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
    keys_read, keys_write = os.pipe()
    screen_read, screen_write = os.pipe()
    produced = threading.Event()
    closing = threading.Event()
    errors = []

    def run():
        try:
            launcher.relay(near, keys_read, screen_write)
        except ValueError as error:
            if not closing.is_set() or "truncated console frame" not in str(error):
                errors.append(error)

    def produce():
        try:
            far.sendall(launcher.encode_output(b"x" * (1024 * 1024)))
            produced.set()
        except OSError:
            pass

    relay = threading.Thread(target=run, daemon=True)
    producer = threading.Thread(target=produce, daemon=True)
    relay.start()
    producer.start()
    try:
        assert select.select([screen_read], [], [], 2)[0], "no real display output arrived"
        os.write(keys_write, b"keys while display blocked")
        reader, events = launcher.FrameReader(), []
        far.settimeout(2)
        while not events:
            events.extend(reader.feed(far.recv(4096)))
        assert events == [("input", b"keys while display blocked")]
        assert not produced.is_set(), "blocked display was drained into an unbounded output queue"
    finally:
        closing.set()
        far.close()
        os.close(screen_read)
        relay.join(timeout=2)
        near.close()
        producer.join(timeout=2)
        for fd in (keys_read, keys_write, screen_write):
            os.close(fd)
    assert not relay.is_alive() and not producer.is_alive()
    assert not errors, errors
