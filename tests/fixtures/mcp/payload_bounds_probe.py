"""Run inside a real guest; print summaries rather than copying tool bodies out."""

import json
import os
import select
import subprocess
import sys
import time


def main():
    payload_limit, id_limit, text_bytes = map(int, sys.argv[1:])
    json_id = "\0" * id_limit
    sse_id = "\x01" * id_limit
    messages = [
        {
            "jsonrpc": "2.0",
            "id": "init",
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "payload-boundary", "version": "1"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
    ]
    for mode, identity in [("json", json_id), ("sse", sse_id)]:
        for request_id, arguments in [
            (identity, {"mode": mode}),
            (f"over-{mode}", {"mode": mode, "oversized": True}),
            (f"after-{mode}", {"mode": "echo"}),
        ]:
            messages.append(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "tools/call",
                    "params": {"name": "payload__boundary", "arguments": arguments},
                }
            )
    oversized_input = {
        "jsonrpc": "2.0",
        "id": "never-forward",
        "method": "tools/call",
        "params": {
            "name": "payload__boundary",
            "arguments": {"padding": "x" * payload_limit},
        },
    }
    proc = subprocess.Popen(
        ["/run/capsem-mcp-server"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    buffered = bytearray()
    responses = []
    try:
        input_pipe, output_pipe = proc.stdin, proc.stdout
        assert input_pipe is not None and output_pipe is not None

        def send(raw):
            input_pipe.write(raw + b"\n")
            input_pipe.flush()

        def read_reply():
            deadline = time.monotonic() + 15
            while b"\n" not in buffered:
                remaining = deadline - time.monotonic()
                assert (
                    remaining > 0 and select.select([output_pipe], [], [], remaining)[0]
                ), "MCP response timed out"
                chunk = os.read(output_pipe.fileno(), 65536)
                assert chunk, "relay closed while awaiting a response"
                buffered.extend(chunk)
            newline = buffered.index(b"\n")
            line = bytes(buffered[:newline])
            del buffered[: newline + 1]
            reply = json.loads(line)
            responses.append(reply)
            return reply

        send(json.dumps(messages[0]).encode())
        assert "result" in read_reply(), "initialize failed"
        # These errors must be local, and the same relay must remain usable.
        for raw in [
            b"\xff",
            json.dumps(oversized_input, separators=(",", ":")).encode(),
        ]:
            send(raw)
            assert "id" not in read_reply(), "invalid input was forwarded"
        send(json.dumps(messages[1]).encode())
        for message in messages[2:]:
            send(json.dumps(message, separators=(",", ":")).encode())
            reply = read_reply()
            assert reply.get("id") == message["id"], "correlation changed"
        # Each recovery call above starts after its preceding refusal arrived.
        stdout, stderr = proc.communicate(timeout=10)
        assert not stdout.strip() and not buffered, "unexpected extra responses"
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.communicate(timeout=10)
    assert proc.returncode == 0, stderr.decode(errors="replace")
    known = {response["id"]: response for response in responses if "id" in response}
    refusals = [response for response in responses if "id" not in response]
    assert len(refusals) == 2, f"expected two local refusals, got {len(refusals)}"
    assert all("not forwarded" in response["error"]["message"] for response in refusals)
    assert "never-forward" not in known
    assert len(known) == 7, f"expected all seven correlated responses, got {len(known)}"
    expected = "r" * text_bytes
    for mode, identity in [("json", json_id), ("sse", sse_id)]:
        exact = known[identity]
        assert "error" not in exact, exact.get("error")
        assert exact["result"]["content"] == [{"type": "text", "text": expected}], (
            f"changed {mode} result"
        )
        error = known[f"over-{mode}"]["error"]["message"]
        assert "exceeds" in error and "tool may already have run" in error, error
        assert known[f"after-{mode}"]["result"]["content"] == [
            {"type": "text", "text": "after refusal"}
        ]
    print(
        json.dumps(
            {
                "boundary_bytes": text_bytes,
                "complete_modes": ["json", "sse"],
                "local_refusals": len(refusals),
            }
        )
    )


if __name__ == "__main__":
    main()
