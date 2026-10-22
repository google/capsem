"""The terminal benchmark must exercise the current public stream protocol."""

import json
import runpy
from pathlib import Path
from types import SimpleNamespace

import msgpack
import psutil
import pytest

from benchmarks.helpers.stream_measurements import measure, service_gateway

ROOT = Path(__file__).resolve().parents[1]
GOLDEN = json.loads((ROOT / "sdk/specification/stream-v3.json").read_text())


def collector():
    return runpy.run_path(str(ROOT / "benchmarks/collectors/stream"))


class Socket:
    def __init__(self, *frames):
        self.frames = iter(frames)
        self.sent = []

    def recv(self, timeout):
        assert timeout > 0
        return next(self.frames)

    def send(self, frame):
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


def test_stream_open_uses_header_authentication_and_focuses_terminal():
    module = collector()
    started = next(
        row for row in GOLDEN["server"] if row["expect"].get("status") == {"type": "started"}
    )
    socket = Socket(bytes(started["frame"]))
    calls = []

    def connect(url, **kwargs):
        calls.append((url, kwargs))
        return socket

    module["open_stream"].__globals__["connect"] = connect
    module["open_stream"](
        SimpleNamespace(port=12345, token="private-token"),
        "session-id",
        {"type": "start", "kind": "terminal"},
    )
    url, options = calls[0]
    assert url == "ws://127.0.0.1:12345/vms/session-id/stream"
    assert options["additional_headers"] == {"Authorization": "Bearer private-token"}
    assert options["subprotocols"] == [GOLDEN["subprotocol"]]
    assert socket.sent == [
        module["control"]({"type": "start", "kind": "terminal"}),
        module["control"]({"type": "focus", "cols": 80, "rows": 24}),
    ]


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


def test_endpoint_reuses_the_service_owned_companion(tmp_path):
    (tmp_path / "gateway.port").write_text("12345")
    (tmp_path / "gateway.token").write_text("private-token")
    endpoint = service_gateway(SimpleNamespace(tmp_dir=tmp_path))
    assert endpoint.port == 12345 and endpoint.token == "private-token"


def test_phase_resources_are_sampled_through_the_existing_tree_owner():
    metrics = {}
    assert measure(metrics, "idle", psutil.Process(), lambda: "result") == "result"
    assert metrics["idle.wall_s"]["samples"][0] >= 0
    assert metrics["idle.resources.total.peak_rss_bytes"]["samples"][0] > 0
    assert metrics["idle.resources.total.cpu_s"]["unit"] == "seconds"
