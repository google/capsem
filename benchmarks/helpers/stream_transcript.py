"""Bounded real rotated transcript reads after the owning producer drains."""
from __future__ import annotations

import base64
import json

from helpers.http_transport import Transport
from helpers.service import vm_session_dir

from .route_measurements import Observations, route_metrics, validate_receipt

MARKER = b"CAPSEM_TRANSCRIPT_42_DONE"
COMMAND = b"printf 'CAPSEM_TRANSCRIPT_%s_DONE\\n' 42\n"


def validate_transcript(body: bytes, *, max_bytes: int, tail_lines: int) -> None:
    assert len(body) <= max_bytes
    payload = json.loads(body)
    output = base64.b64decode(payload["content"], validate=True)
    assert type(payload["bytes"]) is int and payload["bytes"] == len(output) > 0
    assert output.count(MARKER) == 1, "missing/duplicate known foreground marker"
    # Match the route's LF tail: CR rewrites stay within a terminal line.
    # A final LF terminates its line; an unterminated final line still counts.
    lines = output.count(b"\n") + int(not output.endswith(b"\n"))
    assert lines <= tail_lines


def transcript_samples(service, gateway, vm: str, cfg: dict, samples: int) -> dict:
    # The terminal transfer itself rotates the real producer's file. Never
    # fabricate/rewrite PTY frames or infer rotation from returned tail alone.
    directory = vm_session_dir(service.tmp_dir, service.client(), vm)
    assert (directory / cfg["transcript_rotated_path"]).stat().st_size >= cfg["transcript_rotation_min_bytes"]
    metrics = {}
    path = f"/vms/{vm}/history/transcript?tail_lines={cfg['transcript_tail_lines']}"
    for label in ("service", "gateway"):
        def factory(label=label):
            return Transport(socket_path=str(service.uds_path)) if label == "service" else Transport(host="127.0.0.1", port=gateway.port)
        headers = {} if label == "service" else {"Authorization": f"Bearer {gateway.token}"}
        def request(transport, headers=headers):
            receipt = transport.measure_request("GET", path, headers=headers, timeout=cfg["events_timeout_seconds"], max_response_bytes=cfg["transcript_max_bytes"])
            validate_receipt(receipt)
            assert receipt.status == 200
            validate_transcript(receipt.body, max_bytes=cfg["transcript_max_bytes"], tail_lines=cfg["transcript_tail_lines"])
            return receipt
        warm, cold = Observations(), Observations()
        transport = factory()
        try:
            request(transport)
            for _ in range(samples):
                warm.append(request(transport))
        finally:
            transport.close()
        for _ in range(samples):
            transport = factory()
            try:
                cold.append(request(transport))
            finally:
                transport.close()
        metrics.update(route_metrics(label, "GET /vms/{id}/history/transcript [rotated-tail]", warm, cold, {}, expected=samples))
    return metrics
