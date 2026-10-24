"""The terminal benchmark must exercise the current public stream protocol."""

import inspect
import json
import runpy
from contextlib import nullcontext
from pathlib import Path
from types import SimpleNamespace

import msgpack
import psutil
import pytest

from benchmarks.helpers import stream_measurements
from benchmarks.helpers.stream_measurements import measure, service_gateway

ROOT = Path(__file__).resolve().parents[1]
GOLDEN = json.loads((ROOT / "sdk/specification/stream-v3.json").read_text())


def collector():
    return runpy.run_path(str(ROOT / "benchmarks/collectors/stream"))


class Socket:
    def __init__(self, *frames):
        self.frames = iter(frames)
        self.sent = []
        self.entered = False
        self.closed = False

    def __enter__(self):
        self.entered = True
        return self

    def __exit__(self, *_):
        self.closed = True

    def recv(self, timeout):
        assert timeout > 0
        return next(self.frames)

    def send(self, frame):
        assert self.entered, "connect must be entered before it owns a connection"
        self.sent.append(frame)


def test_benchmark_encodes_the_shared_rust_client_records():
    module = collector()
    assert module["SUBPROTOCOL"] == GOLDEN["subprotocol"]
    for row in GOLDEN["client"]:
        encoded = (
            module["control"](row["control"])
            if "control" in row
            else module["stdin"](bytes(row["stdin"]))
        )
        # Named maps have no field-order contract; binary payloads do.
        assert msgpack.unpackb(encoded) == msgpack.unpackb(bytes(row["frame"]))


def test_benchmark_decodes_shared_server_records_and_rejects_invalid_frames():
    module = collector()
    for row in GOLDEN["server"]:
        socket = Socket(bytes(row["frame"]))
        if row["expect"]["kind"] == "invalid":
            with pytest.raises((RuntimeError, ValueError)):
                module["next_frame"](socket, 1)
        else:
            kind, payload = module["next_frame"](socket, 1)
            if row["expect"]["kind"] == "status":
                assert kind == module["STATUS"]
                assert payload == row["expect"]["status"]
            else:
                assert kind in (module["STDOUT"], module["STDERR"])
                assert payload == bytes(row["expect"]["bytes"])


def test_stream_open_uses_header_authentication_and_focuses_terminal(monkeypatch):
    module = collector()
    started = next(
        row for row in GOLDEN["server"] if row["expect"].get("status") == {"type": "started"}
    )
    socket = Socket(bytes(started["frame"]))
    calls = []

    def connect(url, **kwargs):
        calls.append((url, kwargs))
        return socket

    monkeypatch.setitem(inspect.unwrap(module["open_stream"]).__globals__,"connect",connect)
    with module["open_stream"](
        SimpleNamespace(port=12345, token="private-token"),
        "session-id",
        {"type": "start", "kind": "terminal"},
    ) as opened:
        assert opened is socket
    url, options = calls[0]
    assert url == "ws://127.0.0.1:12345/vms/session-id/stream"
    assert options["additional_headers"] == {"Authorization": "Bearer private-token"}
    assert options["subprotocols"] == [GOLDEN["subprotocol"]]
    assert socket.sent == [
        module["control"]({"type": "start", "kind": "terminal"}),
        module["control"]({"type": "focus", "cols": 80, "rows": 24}),
    ]
    assert socket.closed


def test_failed_start_releases_the_connection(monkeypatch):
    module = collector()
    socket = Socket(b"not a messagepack frame")
    monkeypatch.setitem(inspect.unwrap(module["open_stream"]).__globals__,"connect",lambda *_, **__: socket)
    with (
        pytest.raises((ValueError, RuntimeError)),
        module["open_stream"](
            SimpleNamespace(port=12345, token="private-token"),
            "session-id",
            {"type": "start", "kind": "terminal"},
        ),
    ):
        raise AssertionError("invalid start must not reach benchmark workload")
    assert socket.closed


def test_terminal_metadata_does_not_end_an_output_measurement():
    module = collector()
    terminal = next(
        row for row in GOLDEN["server"] if row["expect"].get("status", {}).get("type") == "terminal"
    )
    output = next(row for row in GOLDEN["server"] if row["expect"].get("bytes") == list(b"hello\n"))
    assert (
        module["read_until"](Socket(bytes(terminal["frame"]), bytes(output["frame"])), b"hello", 1)
        == 6
    )


def test_readiness_uses_foreground_output_instead_of_prompt_format():
    module = collector()
    socket = Socket(msgpack.packb({"stdout": b"\x1b[0mBENCH_READY_42_DONE\r\n"}, use_bin_type=True))
    socket.entered = True
    module["ready_terminal"](socket)
    assert b"printf" in msgpack.unpackb(socket.sent[0])["stdin"]


def test_endpoint_reuses_the_service_owned_companion(tmp_path):
    (tmp_path / "gateway.port").write_text("12345")
    (tmp_path / "gateway.token").write_text("private-token")
    (tmp_path / "gateway.pid").write_text("4242")
    endpoint = service_gateway(SimpleNamespace(tmp_dir=tmp_path))
    assert endpoint.port == 12345 and endpoint.token == "private-token"
    assert endpoint.pid == 4242


def test_endpoint_waits_for_prepared_empty_files_to_be_published(tmp_path, monkeypatch):
    for name in ("port", "token", "pid"):
        (tmp_path / f"gateway.{name}").write_text("")

    def publish(_):
        for name, value in (("port", "12345"), ("token", "private-token"), ("pid", "4242")):
            (tmp_path / f"gateway.{name}").write_text(value)

    monkeypatch.setattr(stream_measurements, "time", SimpleNamespace(monotonic=lambda: 0, sleep=publish))
    endpoint = service_gateway(SimpleNamespace(tmp_dir=tmp_path))
    assert (endpoint.port, endpoint.token, endpoint.pid) == (12345, "private-token", 4242)


def test_phase_resources_are_sampled_through_the_existing_tree_owner():
    metrics = {}
    assert measure(metrics, "idle", psutil.Process(), lambda: "result") == "result"
    assert metrics["idle.wall_s"]["samples"][0] >= 0
    assert metrics["idle.resources.total.peak_rss_bytes"]["samples"][0] > 0
    assert metrics["idle.resources.total.cpu_s"]["unit"] == "seconds"


def test_terminal_distinguishes_source_processing_from_projected_wire_rate(monkeypatch):
    module = collector()
    function = module["terminal_output"]
    socket = Socket()
    socket.entered = True
    ticks = iter((10.0, 12.0))
    monkeypatch.setitem(function.__globals__, "open_stream", lambda *_: nullcontext(socket))
    monkeypatch.setitem(function.__globals__, "ready_terminal", lambda _: None)
    monkeypatch.setitem(function.__globals__, "read_until", lambda *_, **__: 1_000_000)
    monkeypatch.setitem(function.__globals__, "time", SimpleNamespace(monotonic=lambda: next(ticks)))
    source_mbps, projected_mbps = function(None, "session-id")
    assert source_mbps == 67.108864  # 16 MiB input completed in two seconds.
    assert projected_mbps == 4.0  # Screen projection transmitted only 1 MB.
