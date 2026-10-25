"""Shared original stream-v3 benchmark client and bounded marker observations."""
from __future__ import annotations

import contextlib
import json
import math
import time
from pathlib import Path

import msgpack
from websockets.sync.client import ClientConnection, connect
from websockets.typing import Subprotocol

from .stream_measurements import Gateway

ROOT = Path(__file__).resolve().parents[2]
WIRE = json.loads((ROOT / "sdk/specification/stream-v3.json").read_text())
SUBPROTOCOL = Subprotocol(WIRE["subprotocol"])
STDOUT, STDERR, STATUS = "stdout", "stderr", "status"


def control(message: dict) -> bytes:
    return msgpack.packb({"control": message}, use_bin_type=True)


def stdin(data: bytes) -> bytes:
    return msgpack.packb({"stdin": data}, use_bin_type=True)


def remaining(deadline,default):
    if deadline is not None and not math.isfinite(deadline):
        raise ValueError('stream deadline must be finite')
    value = default if deadline is None else min(default,deadline-time.monotonic())
    if not math.isfinite(value) or value <= 0:
        raise TimeoutError('stream total deadline expired')
    return value


@contextlib.contextmanager
def open_stream(gateway: Gateway, vm: str, start: dict, *, deadline=None, close_timeout=10):
    timeout = remaining(deadline,30)
    close = remaining(deadline,close_timeout)
    url = f"ws://127.0.0.1:{gateway.port}/vms/{vm}/stream"
    with connect(
        url,
        additional_headers={"Authorization": f"Bearer {gateway.token}"},
        subprotocols=[SUBPROTOCOL],
        max_size=WIRE["max_frame_bytes"],
        open_timeout=timeout,
        close_timeout=close,
    ) as stream:
        stream.send(control(start))
        status = next_frame(stream, timeout=remaining(deadline,60))
        if status != (STATUS, {"type": "started"}):
            raise RuntimeError(f"stream did not start: {status}")
        if start["kind"] == "terminal":
            stream.send(control({"type": "focus", "cols": 80, "rows": 24}))
        yield stream


def next_frame(stream: ClientConnection, timeout: float) -> tuple[str, object]:
    frame = stream.recv(timeout=timeout)
    if not isinstance(frame, bytes) or not frame:
        raise RuntimeError(f"unexpected stream frame: {frame!r}")
    if len(frame) > WIRE["max_frame_bytes"]:
        raise RuntimeError("stream frame exceeded the shared budget")
    value = msgpack.unpackb(frame, raw=False)
    if not isinstance(value, dict) or len(value) != 1:
        raise RuntimeError("stream frame must contain one named record")
    channel, payload = next(iter(value.items()))
    if channel == STATUS and isinstance(payload, dict):
        return STATUS, payload
    if channel in (STDOUT, STDERR) and isinstance(payload, bytes):
        return channel, payload
    raise RuntimeError(f"unexpected stream record: {channel}")


def read_until(stream: ClientConnection, marker: bytes | tuple[bytes, ...], timeout: float,
               *, max_frames: int | None=None, max_bytes: int | None=None) -> int:
    """Bytes of output up to and including `marker` (or any of several)."""
    if not math.isfinite(timeout) or timeout <= 0 or any(
        value is not None and (type(value) is not int or value <= 0) for value in (max_frames,max_bytes)):
        raise ValueError('stream probe budget invalid')
    markers = marker if isinstance(marker, tuple) else (marker,)
    deadline = time.monotonic() + timeout
    seen = b""
    total = 0
    frames = 0
    while not any(candidate in seen for candidate in markers):
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("terminal marker deadline exceeded")
            if max_frames is not None and frames >= max_frames:
                raise ValueError('stream frame budget exhausted')
            channel, payload = next_frame(stream, remaining)
            frames += 1
        except TimeoutError as error:
            raise TimeoutError(
                f"terminal receive deadline: lacks {marker!r}; last output: {seen[-500:]!r}"
            ) from error
        if channel == STATUS:
            if isinstance(payload, dict) and payload.get("type") == "terminal":
                continue
            raise RuntimeError(f"stream ended while waiting for {marker!r}: {payload}")
        assert isinstance(payload, bytes)
        total += len(payload)
        if max_bytes is not None and total > max_bytes:
            raise ValueError('stream byte budget exhausted')
        combined = seen + payload
        if any(candidate in combined for candidate in markers):
            return total
        seen = combined[-4096:]
    return total


def ready_terminal(stream: ClientConnection, *, deadline=None, max_frames=None, max_bytes=None) -> None:
    # A canonical screen projection may style or reposition each prompt cell.
    # Use expanded foreground output rather than guessing a prompt's bytes.
    deadline = time.monotonic()+remaining(deadline,30)
    while time.monotonic() < deadline:
        stream.send(stdin(b"printf 'BENCH_READY_%s_DONE\\n' 42\n"))
        try:
            read_until(stream, b"BENCH_READY_42_DONE", timeout=remaining(deadline,2),
                max_frames=max_frames,max_bytes=max_bytes)
            return
        except TimeoutError:
            if time.monotonic() >= deadline:
                raise
    raise TimeoutError('terminal readiness deadline exceeded')
