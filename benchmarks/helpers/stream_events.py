"""JSON WebSocket transport RTT and genuine status-poll transition delivery."""
from __future__ import annotations

import json
import time

from helpers.http_transport import Transport
from websockets.sync.client import connect

from .route_measurements import validate_receipt


def decode_event(frame: object, vm: str) -> str:
    assert isinstance(frame, str), "events must be JSON text, not stream-v3 bytes"
    value = json.loads(frame)
    assert value["type"] == "vm-state-changed"
    payload = value["payload"]
    assert payload["id"] == vm and payload["trigger"] == "status_poll"
    assert payload["state"] in {"Running", "Stopped", "Suspended", "Defunct", "Incompatible"}
    return payload["state"]


def status(transport, gateway, vm: str, expected: str, cfg: dict) -> None:
    receipt = transport.measure_request("GET", "/status", headers={"Authorization": f"Bearer {gateway.token}"},
                                        timeout=cfg["events_timeout_seconds"], max_response_bytes=cfg["events_max_frame_bytes"])
    validate_receipt(receipt)
    assert receipt.status == 200
    payload = json.loads(receipt.body)
    assert payload["service"] == "running"
    rows = [row for row in payload["vms"] if row["id"] == vm]
    assert len(rows) == 1 and rows[0]["status"] == expected, payload


def event_samples(service, gateway, vm: str, cfg: dict, ping_samples: int) -> dict:
    """One disposable owner stops; no initial event or removed-VM event assumed."""
    transport = Transport(host="127.0.0.1", port=gateway.port)
    try:
        # Prime Running BEFORE subscribing; idle subscribe promises no event.
        status(transport, gateway, vm, "Running", cfg)
        with connect(f"ws://127.0.0.1:{gateway.port}/events", additional_headers={"Authorization": f"Bearer {gateway.token}"},
                     max_size=cfg["events_max_frame_bytes"], max_queue=cfg["events_max_frames"], ping_interval=None,
                     open_timeout=cfg["events_open_timeout_seconds"], close_timeout=cfg["events_close_timeout_seconds"]) as socket:
            pings = []
            for index in range(ping_samples):
                started = time.monotonic()
                # websockets matches the Pong payload to this exact Ping.
                if not socket.ping(f"capsem-bench-{index}".encode()).wait(cfg["events_timeout_seconds"]):
                    raise TimeoutError("events Pong missing")
                pings.append((time.monotonic() - started) * 1000)
            stopped_at = time.monotonic()
            stopped = service.client().post(f"/vms/{vm}/stop", {}, timeout=cfg["mutation_timeout_seconds"])
            assert stopped["success"] is True and stopped["persistent"] is True
            polled_at = time.monotonic()
            status(transport, gateway, vm, "Stopped", cfg)
            deadline = polled_at + cfg["events_timeout_seconds"]
            for _ in range(cfg["events_max_frames"]):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("VM transition event deadline exceeded")
                if decode_event(socket.recv(timeout=remaining), vm) == "Stopped":
                    ended = time.monotonic()
                    return {
                        "events.transport_ping_pong_ms": {"unit": "milliseconds", "samples": pings},
                        "events.stop_to_event_ms": {"unit": "milliseconds", "samples": [(ended - stopped_at) * 1000]},
                        "events.status_poll_to_event_ms": {"unit": "milliseconds", "samples": [(ended - polled_at) * 1000]},
                    }
            raise RuntimeError("bounded event frame budget exhausted before Stopped")
    finally:
        transport.close()
