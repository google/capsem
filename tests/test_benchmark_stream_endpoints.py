"""Streaming endpoint evidence separates transport RTT, lifecycle and transcript."""
from __future__ import annotations

import base64
import json

import pytest

from benchmarks.helpers.stream_events import decode_event
from benchmarks.helpers.stream_transcript import MARKER, validate_transcript


def test_transcript_is_nonempty_exact_bounded_foreground_output() -> None:
    output = b"prefix\r\n" + MARKER + b"\r\n$ "
    payload = {"bytes": len(output), "content": base64.b64encode(output).decode()}
    validate_transcript(json.dumps(payload).encode(), max_bytes=1024, tail_lines=16)
    for damaged in ({"bytes": 0, "content": ""}, {**payload, "bytes": len(output) + 1}, {**payload, "content": "!!!!"}, {"bytes": 1, "content": "eA=="}):
        with pytest.raises((AssertionError, ValueError)):
            validate_transcript(json.dumps(damaged).encode(), max_bytes=1024, tail_lines=16)
    with pytest.raises(AssertionError):
        validate_transcript(json.dumps(payload).encode(), max_bytes=4, tail_lines=16)
    with pytest.raises(AssertionError):
        validate_transcript(json.dumps(payload).encode(), max_bytes=1024, tail_lines=1)


def test_events_require_real_vm_transition_after_authoritative_poll() -> None:
    payload = {"type": "vm-state-changed", "payload": {"id": "vm", "state": "Stopped", "trigger": "status_poll"}}
    assert decode_event(json.dumps(payload), "vm") == "Stopped"
    for damaged in ({**payload, "type": "pong"}, {**payload, "payload": {"id": "vm", "state": "Stopped", "trigger": "invented"}}, {**payload, "payload": {"id": "wrong", "state": "Stopped", "trigger": "status_poll"}}):
        with pytest.raises((AssertionError, ValueError)):
            decode_event(json.dumps(damaged), "vm")
    with pytest.raises(AssertionError):
        decode_event(json.dumps(payload).encode(), "vm")


def test_stream_endpoint_bindings_are_not_polled_route_cases() -> None:
    from benchmarks.helpers.stream_measurements import endpoint_bindings, require_endpoint_metrics
    bindings = endpoint_bindings()
    assert set(bindings) == {"GET /vms/{id}/stream", "GET /vms/{id}/history/transcript", "GET /events"}
    with pytest.raises(ValueError, match="missing"):
        require_endpoint_metrics({})
    metrics = {key: {"unit": "milliseconds", "samples": [1.0]} for row in bindings.values() for case in row["cases"].values() for key in case["metrics"]}
    from benchmarks.helpers.stream_measurements import stream_config
    cfg = stream_config()
    for row in bindings.values():
        for case in row["cases"].values():
            for key in case["metrics"]:
                metrics[key]["samples"] = [1.0] * cfg[case["sample_key"]]
    require_endpoint_metrics(metrics)
    metrics[next(iter(metrics))]["samples"] = []
    with pytest.raises(ValueError):
        require_endpoint_metrics(metrics)


def test_events_prime_running_then_stop_poll_and_receive_with_bounded_ownership(monkeypatch) -> None:
    from types import SimpleNamespace

    from benchmarks.helpers import stream_events as events
    from benchmarks.helpers.stream_measurements import stream_config
    cfg = stream_config()
    calls = []
    class Transport:
        def __init__(self, **_):
            pass
        def close(self):
            calls.append("http closed")
    class Socket:
        def __enter__(self):
            calls.append("subscribed")
            return self
        def __exit__(self, *_):
            calls.append("ws closed")
        def ping(self, data):
            calls.append(data)
            return SimpleNamespace(wait=lambda timeout: timeout > 0)
        def recv(self, timeout):
            assert timeout > 0
            calls.append("event")
            return json.dumps({"type": "vm-state-changed", "payload": {"id": "vm", "state": "Stopped", "trigger": "status_poll"}})
    def connect(url, **options):
        assert url.endswith("/events")
        assert options["additional_headers"] == {"Authorization": "Bearer test-only"}
        assert options["max_size"] == cfg["events_max_frame_bytes"] and options["ping_interval"] is None
        return Socket()
    def stop(path, body, timeout):
        assert path == "/vms/vm/stop" and body == {} and timeout == cfg["mutation_timeout_seconds"]
        calls.append("stopped")
        return {"success": True, "persistent": True}
    monkeypatch.setattr(events, "Transport", Transport)
    monkeypatch.setattr(events, "connect", connect)
    monkeypatch.setattr(events, "status", lambda _, __, ___, state, ____: calls.append(state))
    metrics = events.event_samples(SimpleNamespace(client=lambda: SimpleNamespace(post=stop)), SimpleNamespace(port=12345, token="test-only"), "vm", cfg, 2)
    assert calls == ["Running", "subscribed", b"capsem-bench-0", b"capsem-bench-1", "stopped", "Stopped", "event", "ws closed", "http closed"]
    assert len(metrics["events.transport_ping_pong_ms"]["samples"]) == 2
    assert len(metrics["events.stop_to_event_ms"]["samples"]) == 1
    assert metrics["events.stop_to_event_ms"]["samples"][0] >= metrics["events.status_poll_to_event_ms"]["samples"][0] >= 0


def test_missing_pong_prevents_transition_and_releases_both_transports(monkeypatch) -> None:
    from types import SimpleNamespace

    from benchmarks.helpers import stream_events as events
    from benchmarks.helpers.stream_measurements import stream_config
    closed = []
    class Socket:
        def __enter__(self):
            return self
        def __exit__(self, *_):
            closed.append("ws")
        def ping(self, _):
            return SimpleNamespace(wait=lambda _: False)
    monkeypatch.setattr(events, "Transport", lambda **_: SimpleNamespace(close=lambda: closed.append("http")))
    monkeypatch.setattr(events, "status", lambda *_: None)
    monkeypatch.setattr(events, "connect", lambda *_, **__: Socket())
    def forbidden():
        raise AssertionError("no VM transition before matched Pong")
    with pytest.raises(TimeoutError, match="Pong"):
        events.event_samples(SimpleNamespace(client=forbidden), SimpleNamespace(port=12345, token="test-only"), "vm", stream_config(), 1)
    assert closed == ["ws", "http"]


def test_rotation_budget_matches_current_owner_and_real_terminal_output() -> None:
    import math
    import re
    from pathlib import Path

    from benchmarks.helpers.stream_measurements import stream_config
    cfg = stream_config()
    root = Path(__file__).resolve().parents[1]
    source = (root / "crates/capsem-core/src/pty_log.rs").read_text()
    value = re.search(r"const DEFAULT_MAX_BYTES: u64 = ([0-9 *]+);", source)
    assert value is not None
    threshold = math.prod(int(part.strip()) for part in value[1].split("*"))
    assert cfg["transcript_rotation_min_bytes"] == threshold
    assert cfg["terminal_bytes"] * 4 // 3 > threshold
    assert cfg["transcript_rotated_path"] == "owner/logs/pty.log.1"
    assert 'OWNER_STATE_DIR: &str = "owner"' in (root / "crates/capsem-core/src/session/mod.rs").read_text()
    assert 'LOG_DIR: &str = "logs"' in (root / "crates/capsem-core/src/session/logs.rs").read_text()


def test_typed_stream_budgets_and_binding_keys_are_finite_and_explicit() -> None:
    from capsem_builder.gate.benchmarkstreamschema import StreamCaseEvidence, StreamConfig
    from pydantic import ValidationError

    from benchmarks.helpers.stream_measurements import stream_config
    cfg = stream_config()
    StreamConfig.model_validate(cfg)
    for field in ("events_timeout_seconds", "events_max_frame_bytes", "events_max_frames", "transcript_max_bytes", "transcript_samples"):
        with pytest.raises(ValidationError):
            StreamConfig.model_validate({**cfg, field: 0})
    with pytest.raises(ValidationError):
        StreamCaseEvidence.model_validate({"sample_key": "invented", "metrics": ["made.up"]})


def test_transcript_strict_receipts_validate_original_body_and_count_all_samples(monkeypatch, tmp_path) -> None:
    from types import SimpleNamespace

    from helpers.http_measurement import MeasuredResponse

    from benchmarks.helpers import stream_transcript as transcript
    from benchmarks.helpers.stream_measurements import stream_config
    cfg = {**stream_config(), "transcript_rotation_min_bytes": 1}
    path = tmp_path / cfg["transcript_rotated_path"]
    path.parent.mkdir(parents=True)
    path.write_bytes(b"x")
    calls = []
    output = MARKER + b"\r\n"
    body = json.dumps({"content": base64.b64encode(output).decode(), "bytes": len(output)}).encode()
    class Transport:
        def __init__(self, **kwargs):
            self.kwargs = kwargs
        def measure_request(self, method, target, **kwargs):
            assert method == "GET" and target == "/vms/vm/history/transcript?tail_lines=16"
            assert kwargs["max_response_bytes"] == cfg["transcript_max_bytes"] and kwargs["timeout"] > 0
            calls.append((self.kwargs, kwargs["headers"]))
            return MeasuredResponse(200, {}, body, 1, 2, 3)
        def close(self):
            pass
    monkeypatch.setattr(transcript, "Transport", Transport)
    monkeypatch.setattr(transcript, "vm_session_dir", lambda *_: tmp_path)
    service = SimpleNamespace(tmp_dir=tmp_path, uds_path=tmp_path / "service.sock", client=lambda: None)
    gateway = SimpleNamespace(port=12345, token="test-only")
    metrics = transcript.transcript_samples(service, gateway, "vm", cfg, 2)
    assert len(calls) == 10 and len(metrics) == 12
    assert all(len(row["samples"]) == 2 for row in metrics.values())
    assert all(headers == {} for _, headers in calls[:5])
    assert all(headers == {"Authorization": "Bearer test-only"} for _, headers in calls[5:])
    path.unlink()
    with pytest.raises(FileNotFoundError):
        transcript.transcript_samples(service, gateway, "vm", cfg, 2)


def test_collector_orders_genuine_marker_then_stop_events_then_drained_transcript(monkeypatch, tmp_path, capsys) -> None:
    import runpy
    import sys
    from contextlib import nullcontext
    from pathlib import Path
    from types import SimpleNamespace

    from benchmarks.helpers.route_measurements import Observations, route_metrics
    from benchmarks.helpers.stream_transcript import COMMAND
    module = runpy.run_path(str(Path(__file__).resolve().parents[1] / "benchmarks/collectors/stream"))
    calls = []
    cfg = module["CONFIG"]
    service = SimpleNamespace(proc=SimpleNamespace(pid=10), tmp_dir=tmp_path, start=lambda: calls.append("service"), stop=lambda: calls.append("cleanup"))
    socket = SimpleNamespace(send=lambda data: calls.append(data))
    def events(*_):
        calls.append("real stop and status event")
        return {"events.transport_ping_pong_ms": {"unit": "milliseconds", "samples": [0.1] * cfg["quick_events_ping_samples"]},
                "events.stop_to_event_ms": {"unit": "milliseconds", "samples": [0.2]},
                "events.status_poll_to_event_ms": {"unit": "milliseconds", "samples": [0.1]}}
    def transcript(*_):
        calls.append("drained transcript")
        observations = Observations([1.0] * cfg["quick_transcript_samples"], [0.1] * cfg["quick_transcript_samples"], [0.2] * cfg["quick_transcript_samples"])
        return {key: value for label in ("service", "gateway") for key, value in route_metrics(label, "GET /vms/{id}/history/transcript [rotated-tail]", observations, observations, {}, expected=cfg["quick_transcript_samples"]).items()}
    def read_marker(_, marker, timeout):
        assert marker == MARKER and timeout > 0
        calls.append("foreground marker")
    for key, value in {
        "ServiceInstance": lambda: service, "service_gateway": lambda _: None,
        "psutil": SimpleNamespace(Process=lambda _: None),
        "image_session": lambda *_: nullcontext("vm"),
        "measure": lambda _, __, ___, operation: operation(),
        "time": SimpleNamespace(sleep=lambda _: None),
        "keystroke_echo": lambda *args: [0.1] * cfg["quick_keystrokes"],
        "resize_echo": lambda *_: [0.1] * cfg["resize_samples"],
        "terminal_output": lambda *_: (1.0, 2.0), "exec_output": lambda *_: (1.0, 2.0),
        "open_stream": lambda *_: nullcontext(socket), "ready_terminal": lambda _: None,
        "read_until": read_marker, "event_samples": events, "transcript_samples": transcript,
    }.items():
        monkeypatch.setitem(module["main"].__globals__, key, value)
    monkeypatch.setattr(sys, "argv", ["stream", "--quick"])
    assert module["main"]() == 0
    assert calls == ["service", module["stdin"](COMMAND), "foreground marker", "real stop and status event", "drained transcript", "cleanup"]
    assert json.loads(capsys.readouterr().out)["metrics"]


def test_event_frame_budget_never_accepts_repeated_wrong_state(monkeypatch) -> None:
    from types import SimpleNamespace

    from benchmarks.helpers import stream_events as events
    from benchmarks.helpers.stream_measurements import stream_config
    cfg = {**stream_config(), "events_max_frames": 2}
    seen = []
    closed = []
    class Socket:
        def __enter__(self):
            return self
        def __exit__(self, *_):
            closed.append("ws")
        def ping(self, _):
            return SimpleNamespace(wait=lambda _: True)
        def recv(self, timeout):
            assert timeout > 0
            seen.append(True)
            return json.dumps({"type": "vm-state-changed", "payload": {"id": "vm", "state": "Running", "trigger": "status_poll"}})
    monkeypatch.setattr(events, "Transport", lambda **_: SimpleNamespace(close=lambda: closed.append("http")))
    monkeypatch.setattr(events, "status", lambda *_: None)
    monkeypatch.setattr(events, "connect", lambda *_, **__: Socket())
    service = SimpleNamespace(client=lambda: SimpleNamespace(post=lambda *_, **__: {"success": True, "persistent": True}))
    with pytest.raises(RuntimeError, match="frame budget"):
        events.event_samples(service, SimpleNamespace(port=12345, token="test-only"), "vm", cfg, 1)
    assert len(seen) == cfg["events_max_frames"] and closed == ["ws", "http"]
