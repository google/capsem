"""Black-box model ledger checks for Ironbank tests."""

from __future__ import annotations

from contextlib import closing
from dataclasses import dataclass
import json
import re
import sqlite3
import time
from pathlib import Path
from typing import Any

from ironbank.model_pricing import assert_model_call_price


@dataclass(frozen=True)
class ModelLedgerSpec:
    input: str
    reasoning: str
    output: str
    tool_call_name: str
    call_args: dict[str, Any]
    call_response: str
    provider: str
    domain: str
    path: str
    model: str
    credential_provider: str | None = None
    credential_source: str | None = None


@dataclass(frozen=True)
class ModelLedgerRun:
    db_path: Path
    upstream_transcript_path: Path
    log_paths: tuple[Path, ...]
    raw_secrets: tuple[str, ...] = ()
    expected_credential_ref: str | None = None


@dataclass(frozen=True)
class ModelLedgerTurn:
    input: str
    reasoning: str
    output: str
    tool_call_name: str
    call_args: dict[str, Any]
    call_response: str
    file_path: str
    file_content: str
    call_id: str | None = None


@dataclass(frozen=True)
class TwoTurnModelLedgerSpec:
    provider: str
    domain: str
    path: str
    model: str
    dns_qname: str
    dns_ip: str
    turns: tuple[ModelLedgerTurn, ModelLedgerTurn]
    credential_provider: str | None = None


def assert_model_ledger_exchange(spec: ModelLedgerSpec, run: ModelLedgerRun) -> None:
    """Assert one model exchange from upstream truth through the Capsem ledger.

    The spec contains only the semantic facts the fixture intentionally asks
    for. Everything else is derived from the upstream transcript and DB.
    """

    with closing(_connect(run.db_path)) as conn:
        upstream_records = _load_upstream_records(run.upstream_transcript_path, spec.path)
        assert upstream_records, f"no upstream records for {spec.path}"
        assert all(row["path"] == spec.path for row in upstream_records)
        assert all(row["status"] == 200 for row in upstream_records)
        assert all(row["method"] == "POST" for row in upstream_records)

        upstream_inputs = "\n".join(row["request_body"] for row in upstream_records)
        upstream_outputs = "\n".join(row["response_body"] for row in upstream_records)
        assert spec.input in upstream_inputs
        assert spec.output in upstream_outputs
        if spec.reasoning:
            assert spec.reasoning in upstream_outputs
        assert spec.tool_call_name in upstream_outputs
        for key in spec.call_args:
            assert key in upstream_outputs
        command = (
            spec.call_args.get("cmd")
            or spec.call_args.get("command")
            or spec.call_args.get("CommandLine")
        )
        if isinstance(command, str):
            assert Path(command.rsplit(">", 1)[-1].strip()).name in upstream_outputs
        assert spec.call_response in upstream_inputs

        expected_usage = [_usage_from_upstream(row) for row in upstream_records]
        expected_usage = [usage for usage in expected_usage if usage is not None]
        assert expected_usage, f"upstream transcript lacks usage for {spec.path}"

        model_rows = _wait_for_rows(
            conn,
            """
            SELECT *
            FROM model_calls
            WHERE provider = ? AND path = ? AND model = ?
            ORDER BY id
            """,
            (spec.provider, spec.path, spec.model),
            len(expected_usage),
            label=f"model_calls for {spec.provider} {spec.path}",
        )
        assert len(model_rows) >= len(expected_usage), (
            f"model_calls missing rows for {spec.provider} {spec.path}: "
            f"rows={len(model_rows)} usage={len(expected_usage)}"
        )
        model_rows = model_rows[-len(expected_usage) :]

        for row, usage in zip(model_rows, expected_usage, strict=True):
            _assert_event_id(row["event_id"])
            assert row["provider"] == spec.provider
            assert row["path"] == spec.path
            assert row["model"] == spec.model
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["input_tokens"] == usage["input_tokens"], dict(row)
            assert row["output_tokens"] == usage["output_tokens"], dict(row)
            details = json.loads(row["usage_details"] or "{}")
            assert details.get("thinking", 0) == usage["thinking_tokens"], dict(row)
            assert row["request_bytes"] > 0
            assert row["response_bytes"] > 0
            assert_model_call_price(row)

        final_model = model_rows[-1]
        assert final_model["text_content"] == spec.output, dict(final_model)
        if spec.reasoning:
            assert final_model["thinking_content"] == spec.reasoning, dict(final_model)

        tool_rows = conn.execute(
            """
            SELECT tool_calls.*, model_calls.path AS model_path, model_calls.model AS model_name
            FROM tool_calls
            JOIN model_calls ON model_calls.id = tool_calls.model_call_id
            WHERE tool_calls.provider = ?
              AND tool_calls.tool_name = ?
              AND model_calls.path = ?
              AND model_calls.model = ?
              AND tool_calls.model_call_id IN ({})
            ORDER BY tool_calls.id
            """.format(",".join("?" for _ in model_rows)),
            (
                spec.provider,
                spec.tool_call_name,
                spec.path,
                spec.model,
                *(row["id"] for row in model_rows),
            ),
        ).fetchall()
        assert len(tool_rows) == 1, [dict(row) for row in tool_rows]
        tool_row = tool_rows[0]
        _assert_event_id(tool_row["event_id"])
        assert json.loads(tool_row["arguments"]) == spec.call_args
        assert tool_row["origin"] in {"native", "mcp"}
        assert tool_row["trace_id"]

        response_rows = conn.execute(
            """
            SELECT *
            FROM tool_responses
            WHERE call_id = ?
              AND trace_id = ?
            ORDER BY id
            """,
            (tool_row["call_id"], final_model["trace_id"]),
        ).fetchall()
        assert len(response_rows) == 1, [dict(row) for row in response_rows]
        response_row = response_rows[0]
        assert response_row["is_error"] == 0
        assert response_row["trace_id"] == final_model["trace_id"]
        assert spec.call_response in (response_row["content_preview"] or "")

        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = ? AND path = ?
            ORDER BY id
            """,
            (spec.domain, spec.path),
        ).fetchall()
        assert len(net_rows) >= len(upstream_records), [dict(row) for row in net_rows]
        net_rows = net_rows[-len(upstream_records) :]
        for row, upstream in zip(net_rows, upstream_records, strict=True):
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["decision"] == "allowed"
            assert row["bytes_sent"] > 0
            assert row["bytes_received"] > 0
            request_preview = row["request_body_preview"] or ""
            response_preview = row["response_body_preview"] or ""
            upstream_request = upstream["request_body"]
            upstream_response = upstream["response_body"]
            if spec.input in upstream_request:
                assert spec.input in request_preview, dict(row)
            if spec.call_response in upstream_request:
                assert spec.call_response in request_preview, dict(row)
            if spec.tool_call_name in upstream_response:
                assert spec.tool_call_name in response_preview, dict(row)
            if spec.output in upstream_response:
                assert spec.output in response_preview, dict(row)
            if spec.reasoning and spec.reasoning in upstream_response:
                assert spec.reasoning in response_preview, dict(row)

        _assert_security_rows(conn, [row["event_id"] for row in (*model_rows, *net_rows)])
        credential_refs = _assert_brokered_model_credentials(
            conn,
            provider=spec.credential_provider or spec.provider,
            expected_source=spec.credential_source,
            model_rows=model_rows,
            tool_rows=tool_rows,
            response_rows=response_rows,
            net_rows=net_rows,
            raw_secrets=run.raw_secrets,
        )
        _assert_tool_output_file(conn, spec, credential_refs=credential_refs)
        _assert_no_raw_secret_in_db(conn, run.raw_secrets)
    _assert_no_raw_secret_in_logs(run.log_paths, run.raw_secrets)


def assert_two_turn_model_ledger_exchange(
    spec: TwoTurnModelLedgerSpec,
    run: ModelLedgerRun,
) -> None:
    """Assert two full model/tool/file turns with exact ledger cardinality."""

    assert len(spec.turns) == 2
    assert len({turn.file_path for turn in spec.turns}) == 2
    assert len({turn.input for turn in spec.turns}) == 2

    with closing(_connect(run.db_path)) as conn:
        upstream_records = _load_upstream_records(run.upstream_transcript_path, spec.path)
        assert len(upstream_records) == 4, upstream_records
        assert all(row["path"] == spec.path for row in upstream_records)
        assert all(row["status"] == 200 for row in upstream_records)
        assert all(row["method"] == "POST" for row in upstream_records)

        upstream_inputs = "\n".join(row["request_body"] for row in upstream_records)
        upstream_outputs = "\n".join(row["response_body"] for row in upstream_records)
        for turn in spec.turns:
            assert turn.input in upstream_inputs
            assert turn.call_response in upstream_inputs
            assert turn.output in upstream_outputs
            assert turn.reasoning in upstream_outputs
            assert turn.tool_call_name in upstream_outputs
            assert Path(turn.file_path).name in upstream_outputs
            for key in turn.call_args:
                assert key in upstream_outputs

        model_rows = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = ? AND path = ? AND model = ?
            ORDER BY id
            """,
            (spec.provider, spec.path, spec.model),
        ).fetchall()
        assert len(model_rows) == 4, [dict(row) for row in model_rows]
        for row in model_rows:
            _assert_event_id(row["event_id"])
            assert row["provider"] == spec.provider
            assert row["path"] == spec.path
            assert row["model"] == spec.model
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["input_tokens"] > 0, dict(row)
            assert row["output_tokens"] >= 0, dict(row)
            assert row["request_bytes"] > 0
            assert row["response_bytes"] > 0
            assert_model_call_price(row)

        item_rows = conn.execute(
            """
            SELECT *
            FROM model_items
            WHERE provider = ? AND path = ? AND model = ?
            ORDER BY id
            """,
            (spec.provider, spec.path, spec.model),
        ).fetchall()
        assert len(item_rows) == 10, [dict(row) for row in item_rows]
        assert all(row["provider"] == spec.provider for row in item_rows)
        assert all(row["path"] == spec.path for row in item_rows)
        assert all(row["model"] == spec.model for row in item_rows)
        assert all(_is_blake3_ref(row["content_hash"]) for row in item_rows)

        by_trace: dict[str, list[sqlite3.Row]] = {}
        for row in item_rows:
            by_trace.setdefault(row["trace_id"], []).append(row)
        assert len(by_trace) == 2, [dict(row) for row in item_rows]

        tool_rows = conn.execute(
            """
            SELECT *
            FROM tool_calls
            WHERE provider = ? AND tool_name = ?
            ORDER BY id
            """,
            (spec.provider, spec.turns[0].tool_call_name),
        ).fetchall()
        assert len(tool_rows) == 2, [dict(row) for row in tool_rows]
        response_rows = conn.execute(
            "SELECT * FROM tool_responses ORDER BY id"
        ).fetchall()
        assert len(response_rows) == 2, [dict(row) for row in response_rows]

        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = ? AND path = ?
            ORDER BY id
            """,
            (spec.domain, spec.path),
        ).fetchall()
        assert len(net_rows) == 4, [dict(row) for row in net_rows]
        for row in net_rows:
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["decision"] == "allowed"
            assert row["bytes_sent"] > 0
            assert row["bytes_received"] > 0

        credential_refs = _assert_brokered_model_credentials(
            conn,
            provider=spec.credential_provider or spec.provider,
            expected_source=None,
            model_rows=model_rows,
            tool_rows=tool_rows,
            response_rows=response_rows,
            net_rows=net_rows,
            raw_secrets=run.raw_secrets,
        )

        dns_rows = conn.execute(
            """
            SELECT *
            FROM dns_events
            WHERE qname = ?
            ORDER BY id
            """,
            (spec.dns_qname,),
        ).fetchall()
        assert len(dns_rows) == 1, [dict(row) for row in dns_rows]
        dns = dns_rows[0]
        _assert_event_id(dns["event_id"])
        assert dns["qtype"] == 1, dict(dns)
        assert dns["qclass"] == 1, dict(dns)
        assert dns["rcode"] == 0, dict(dns)
        assert dns["decision"] == "allowed", dict(dns)
        assert dns["answer_ip"] == spec.dns_ip == "127.0.0.1", dict(dns)
        assert dns["source_proto"] in {"udp", "tcp"}, dict(dns)

        file_event_ids: list[str] = []
        for turn in spec.turns:
            trace_id = _trace_for_turn(by_trace, turn)
            rows = by_trace[trace_id]
            _assert_trace_items(rows, turn)

            trace_model_calls = [row for row in model_rows if row["trace_id"] == trace_id]
            assert len(trace_model_calls) == 2, [dict(row) for row in model_rows]
            trace_net_rows = [row for row in net_rows if row["trace_id"] == trace_id]
            assert len(trace_net_rows) == 2, [dict(row) for row in net_rows]

            trace_tool_calls = [row for row in tool_rows if row["trace_id"] == trace_id]
            assert len(trace_tool_calls) == 1, [dict(row) for row in tool_rows]
            assert trace_tool_calls[0]["call_id"] == (turn.call_id or trace_tool_calls[0]["call_id"])
            assert json.loads(trace_tool_calls[0]["arguments"]) == turn.call_args
            tool_call_model_ids = {row["model_call_id"] for row in trace_tool_calls}
            assert tool_call_model_ids <= {row["id"] for row in trace_model_calls}, [
                dict(row) for row in trace_tool_calls
            ]

            trace_tool_responses = [
                row for row in response_rows if row["trace_id"] == trace_id
            ]
            assert len(trace_tool_responses) == 1, [dict(row) for row in response_rows]
            assert trace_tool_responses[0]["call_id"] == trace_tool_calls[0]["call_id"]
            assert trace_tool_responses[0]["model_call_id"] == trace_model_calls[-1]["id"], {
                "tool_response": dict(trace_tool_responses[0]),
                "model_calls": [dict(row) for row in trace_model_calls],
            }
            assert turn.call_response in (trace_tool_responses[0]["content_preview"] or "")

            file_row = _assert_created_file_row(
                conn,
                trace_id=trace_id,
                file_path=turn.file_path,
                file_content=turn.file_content,
                credential_refs=credential_refs,
            )
            file_event_ids.append(file_row["event_id"])

        _assert_security_rows(
            conn,
            [row["event_id"] for row in [*model_rows, *net_rows, dns]]
            + file_event_ids,
        )
        _assert_no_raw_secret_in_db(conn, run.raw_secrets)
    _assert_no_raw_secret_in_logs(run.log_paths, run.raw_secrets)


def assert_live_model_ledger_exchange(
    spec: ModelLedgerSpec,
    run: ModelLedgerRun,
    *,
    expected_model_calls: int = 2,
) -> None:
    """Assert one live-provider model exchange through the same ledger contract.

    Live-provider canaries are compatibility diagnostics, not release proof.
    They still owe the same double-entry accounting as hermetic Ironbank:
    semantic client facts in, exact DB/log/security/plugin facts out.
    """

    with closing(_connect(run.db_path)) as conn:
        model_rows = _latest_rows(
            conn,
            """
            SELECT *
            FROM model_calls
            WHERE provider = ? AND path = ? AND model = ?
            ORDER BY id
            """,
            (spec.provider, spec.path, spec.model),
            expected_model_calls,
        )
        assert len(model_rows) == expected_model_calls, [dict(row) for row in model_rows]
        for row in model_rows:
            _assert_event_id(row["event_id"])
            assert row["provider"] == spec.provider
            assert row["path"] == spec.path
            assert row["model"] == spec.model
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["input_tokens"] > 0, dict(row)
            assert row["output_tokens"] >= 0, dict(row)
            assert row["request_bytes"] > 0
            assert row["response_bytes"] > 0
            assert_model_call_price(row)

        final_model = model_rows[-1]
        assert final_model["text_content"] == spec.output, dict(final_model)
        if spec.reasoning:
            assert final_model["thinking_content"] == spec.reasoning, dict(final_model)
        assert spec.input in (model_rows[0]["request_body_preview"] or ""), dict(model_rows[0])

        tool_rows = _latest_rows(
            conn,
            """
            SELECT tool_calls.*, model_calls.path AS model_path, model_calls.model AS model_name
            FROM tool_calls
            JOIN model_calls ON model_calls.id = tool_calls.model_call_id
            WHERE tool_calls.provider = ?
              AND tool_calls.tool_name = ?
              AND model_calls.path = ?
              AND model_calls.model = ?
            ORDER BY tool_calls.id
            """,
            (spec.provider, spec.tool_call_name, spec.path, spec.model),
            1,
        )
        assert len(tool_rows) == 1, [dict(row) for row in tool_rows]
        tool_row = tool_rows[0]
        _assert_event_id(tool_row["event_id"])
        assert json.loads(tool_row["arguments"]) == spec.call_args
        assert tool_row["origin"] in {"native", "mcp"}
        assert tool_row["trace_id"]

        response_rows = _latest_rows(
            conn,
            """
            SELECT *
            FROM tool_responses
            WHERE call_id = ?
            ORDER BY id
            """,
            (tool_row["call_id"],),
            1,
        )
        assert len(response_rows) == 1, [dict(row) for row in response_rows]
        response_row = response_rows[0]
        assert response_row["is_error"] == 0
        assert response_row["model_call_id"] == final_model["id"], {
            "tool_response": dict(response_row),
            "final_model": dict(final_model),
        }
        assert response_row["trace_id"] == final_model["trace_id"]
        assert spec.call_response in (response_row["content_preview"] or "")

        net_rows = _latest_rows(
            conn,
            """
            SELECT *
            FROM net_events
            WHERE domain = ? AND path = ?
            ORDER BY id
            """,
            (spec.domain, spec.path),
            expected_model_calls,
        )
        assert len(net_rows) == expected_model_calls, [dict(row) for row in net_rows]
        for row in net_rows:
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["decision"] == "allowed"
            assert row["bytes_sent"] > 0
            assert row["bytes_received"] > 0
        assert spec.input in (net_rows[0]["request_body_preview"] or ""), dict(net_rows[0])
        assert spec.output in (net_rows[-1]["response_body_preview"] or ""), dict(net_rows[-1])

        _assert_security_rows(conn, [row["event_id"] for row in (*model_rows, *net_rows)])
        credential_refs = _assert_brokered_model_credentials(
            conn,
            provider=spec.credential_provider or spec.provider,
            model_rows=model_rows,
            tool_rows=tool_rows,
            response_rows=response_rows,
            net_rows=net_rows,
            raw_secrets=run.raw_secrets,
        )
        if run.expected_credential_ref is not None:
            assert credential_refs == {run.expected_credential_ref}, {
                "expected": run.expected_credential_ref,
                "actual": sorted(credential_refs),
            }
        _assert_tool_output_file(conn, spec, credential_refs=credential_refs)
        _assert_no_raw_secret_in_db(conn, run.raw_secrets)
    _assert_no_raw_secret_in_logs(run.log_paths, run.raw_secrets)


def _connect(db_path: Path) -> sqlite3.Connection:
    assert db_path.exists(), f"missing session DB: {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _latest_rows(
    conn: sqlite3.Connection,
    query: str,
    params: tuple[Any, ...],
    count: int,
) -> list[sqlite3.Row]:
    rows = _wait_for_rows(conn, query, params, count, label="latest rows")
    return rows[-count:]


def _wait_for_rows(
    conn: sqlite3.Connection,
    query: str,
    params: tuple[Any, ...],
    count: int,
    *,
    label: str,
    timeout_s: float = 15.0,
) -> list[sqlite3.Row]:
    deadline = time.monotonic() + timeout_s
    rows: list[sqlite3.Row] = []
    while time.monotonic() < deadline:
        rows = conn.execute(query, params).fetchall()
        if len(rows) >= count:
            return rows
        time.sleep(0.25)
    rows = conn.execute(query, params).fetchall()
    assert len(rows) >= count, f"{label} missing rows: rows={len(rows)} expected={count}"
    return rows


def _load_upstream_records(path: Path, model_path: str) -> list[dict[str, Any]]:
    assert path.exists(), f"missing upstream transcript: {path}"
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("path") == model_path:
            records.append(record)
    return records


def _usage_from_upstream(row: dict[str, Any]) -> dict[str, int] | None:
    body = row["response_body"]
    content_type = row.get("content_type") or ""
    payloads: list[dict[str, Any]]
    if "text/event-stream" in content_type:
        payloads = [
            json.loads(line.removeprefix("data: "))
            for line in body.splitlines()
            if line.startswith("data: ") and line.removeprefix("data: ") != "[DONE]"
        ]
        response_payloads = [
            payload["response"]
            for payload in payloads
            if isinstance(payload.get("response"), dict)
        ]
        if response_payloads:
            payload = response_payloads[-1]
        elif google_payloads := [
            payload for payload in payloads if isinstance(payload.get("usageMetadata"), dict)
        ]:
            payload = google_payloads[-1]
        else:
            message_start = next(
                (
                    payload["message"]
                    for payload in payloads
                    if payload.get("type") == "message_start"
                    and isinstance(payload.get("message"), dict)
                ),
                {},
            )
            message_delta = next(
                (
                    payload
                    for payload in reversed(payloads)
                    if payload.get("type") == "message_delta"
                    and isinstance(payload.get("usage"), dict)
                ),
                {},
            )
            start_usage = message_start.get("usage") if isinstance(message_start, dict) else {}
            delta_usage = message_delta.get("usage") if isinstance(message_delta, dict) else {}
            if isinstance(start_usage, dict) and isinstance(delta_usage, dict):
                payload = {
                    "usage": {
                        "input_tokens": int(start_usage.get("input_tokens") or 0),
                        "output_tokens": int(delta_usage.get("output_tokens") or 0),
                    }
                }
            else:
                payload = {}
    else:
        payload = json.loads(body)

    usage = payload.get("usage") or payload.get("usageMetadata")
    if not isinstance(usage, dict):
        return None
    input_tokens = (
        usage.get("input_tokens")
        or usage.get("prompt_tokens")
        or usage.get("promptTokenCount")
        or 0
    )
    output_tokens = (
        usage.get("output_tokens")
        or usage.get("completion_tokens")
        or usage.get("candidatesTokenCount")
        or 0
    )
    thinking_tokens = (
        _nested_int(usage, "output_tokens_details", "reasoning_tokens")
        or _nested_int(usage, "completion_tokens_details", "reasoning_tokens")
        or int(usage.get("thinking_tokens") or usage.get("thoughtsTokenCount") or 0)
    )
    return {
        "input_tokens": int(input_tokens),
        "output_tokens": int(output_tokens),
        "thinking_tokens": int(thinking_tokens),
    }


def _nested_int(value: dict[str, Any], key: str, nested_key: str) -> int:
    nested = value.get(key)
    if not isinstance(nested, dict):
        return 0
    return int(nested.get(nested_key) or 0)


def _assert_event_id(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"[0-9a-f]{12}", value), value


def _is_blake3_ref(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"blake3:[0-9a-f]{64}", value) is not None


def _trace_for_turn(
    by_trace: dict[str, list[sqlite3.Row]],
    turn: ModelLedgerTurn,
) -> str:
    matches = [
        trace_id
        for trace_id, rows in by_trace.items()
        if any(turn.input in (row["content"] or "") for row in rows)
        or any(turn.output in (row["content"] or "") for row in rows)
    ]
    assert len(matches) == 1, {
        "turn": turn,
        "rows": [dict(row) for rows in by_trace.values() for row in rows],
    }
    return matches[0]


def _assert_trace_items(rows: list[sqlite3.Row], turn: ModelLedgerTurn) -> None:
    assert sum(row["kind"] == "request" for row in rows) == 1, [dict(row) for row in rows]
    assert sum(row["kind"] == "reasoning" for row in rows) == 1, [dict(row) for row in rows]
    assert sum(row["kind"] == "response" for row in rows) == 1, [dict(row) for row in rows]
    assert sum(row["kind"] == "tool_call" for row in rows) == 1, [dict(row) for row in rows]
    assert sum(row["kind"] == "tool_response" for row in rows) == 1, [dict(row) for row in rows]

    request_row = next(row for row in rows if row["kind"] == "request")
    reasoning_row = next(row for row in rows if row["kind"] == "reasoning")
    response_row = next(row for row in rows if row["kind"] == "response")
    tool_call_row = next(row for row in rows if row["kind"] == "tool_call")
    tool_response_row = next(row for row in rows if row["kind"] == "tool_response")

    assert turn.input in (request_row["content"] or "")
    assert turn.file_path in (request_row["content"] or "")
    assert '"tools"' in (request_row["content"] or "")
    assert turn.tool_call_name in (request_row["content"] or "")
    assert reasoning_row["content"] == turn.reasoning
    assert response_row["content"] == turn.output
    if turn.call_id is not None:
        assert tool_call_row["call_id"] == turn.call_id
        assert tool_response_row["call_id"] == turn.call_id
    assert tool_call_row["tool_name"] == turn.tool_call_name
    assert json.loads(tool_call_row["arguments"]) == turn.call_args
    assert turn.file_path in (tool_call_row["content"] or "")
    assert turn.file_content.strip() in (tool_call_row["content"] or "")
    assert tool_response_row["content"] == turn.call_response


def _assert_security_rows(conn: sqlite3.Connection, event_ids: list[str]) -> None:
    placeholders = ",".join("?" for _ in event_ids)
    rows = conn.execute(
        f"""
        SELECT *
        FROM security_rule_events
        WHERE event_id IN ({placeholders})
        ORDER BY id
        """,
        event_ids,
    ).fetchall()
    assert rows, f"missing security rows for {event_ids}"
    covered = {row["event_id"] for row in rows}
    assert set(event_ids) <= covered
    assert "allow" in {row["rule_action"] for row in rows}
    assert all(json.loads(row["rule_json"]) for row in rows)
    assert all(json.loads(row["event_json"]) for row in rows)


def _assert_brokered_model_credentials(
    conn: sqlite3.Connection,
    *,
    provider: str,
    expected_source: str | None,
    model_rows: list[sqlite3.Row],
    tool_rows: list[sqlite3.Row],
    response_rows: list[sqlite3.Row],
    net_rows: list[sqlite3.Row],
    raw_secrets: tuple[str, ...],
) -> set[str]:
    if not raw_secrets:
        return set()

    credential_refs = {
        row["credential_ref"] for row in net_rows if row["credential_ref"] is not None
    }
    assert len(credential_refs) == 1, [dict(row) for row in net_rows]
    credential_ref = next(iter(credential_refs))
    _assert_credential_ref(credential_ref)
    assert {row["credential_ref"] for row in net_rows} == {credential_ref}, [
        dict(row) for row in net_rows
    ]
    assert {row["credential_ref"] for row in model_rows} == {credential_ref}, [
        dict(row) for row in model_rows
    ]
    assert {row["credential_ref"] for row in tool_rows} == {credential_ref}, [
        dict(row) for row in tool_rows
    ]
    assert {row["credential_ref"] for row in response_rows} == {credential_ref}, [
        dict(row) for row in response_rows
    ]

    rows = conn.execute(
        """
        SELECT *
        FROM substitution_events
        WHERE substitution_ref = ?
        ORDER BY id
        """,
        (credential_ref,),
    ).fetchall()
    assert rows, f"missing substitution_events for {credential_ref}"
    outcomes = {row["outcome"] for row in rows}
    assert {"captured", "brokered"} <= outcomes, [dict(row) for row in rows]
    assert all(row["material_class"] == "credential" for row in rows)
    assert all(row["algorithm"] == "blake3" for row in rows)
    assert all(row["provider"] == provider for row in rows), [dict(row) for row in rows]
    assert all(row["confidence"] is None for row in rows)
    assert all(row["trace_id"] for row in rows)
    captured_sources = {row["source"] for row in rows if row["outcome"] == "captured"}
    expected_sources = {
        "openai": "http.header.authorization",
        "anthropic": "http.header.x-api-key",
        "google": "http.header.x-goog-api-key",
    }
    expected_source = expected_source or expected_sources.get(provider)
    assert expected_source is not None, provider
    assert expected_source in captured_sources, [dict(row) for row in rows]

    return credential_refs


def _assert_credential_ref(value: object) -> None:
    assert isinstance(value, str)
    assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", value), value


def _assert_tool_output_file(
    conn: sqlite3.Connection,
    spec: ModelLedgerSpec,
    *,
    credential_refs: set[str],
) -> None:
    command = (
        spec.call_args.get("cmd")
        or spec.call_args.get("command")
        or spec.call_args.get("CommandLine")
    )
    if not isinstance(command, str):
        return
    match = re.search(r">\s*(/root/[^ ]+)", command)
    if not match:
        return
    path = Path(match.group(1)).name
    deadline = time.monotonic() + 15.0
    rows = []
    while time.monotonic() < deadline:
        rows = conn.execute(
            """
            SELECT *
            FROM fs_events
            WHERE name = ? OR path = ?
            ORDER BY id
            """,
            (path, path),
        ).fetchall()
        if rows:
            break
        time.sleep(0.25)
    assert rows, f"missing fs_events for tool output {path}"
    assert any(row["action"] in {"created", "modified", "export"} for row in rows)
    assert all(row["name"] in {path, None} for row in rows)
    if credential_refs:
        assert any(row["credential_ref"] in credential_refs for row in rows), [
            dict(row) for row in rows
        ]


def _assert_created_file_row(
    conn: sqlite3.Connection,
    *,
    trace_id: str,
    file_path: str,
    file_content: str,
    credential_refs: set[str],
) -> sqlite3.Row:
    path = Path(file_path).name
    deadline = time.monotonic() + 15.0
    rows = []
    while time.monotonic() < deadline:
        rows = conn.execute(
            """
            SELECT *
            FROM fs_events
            WHERE action = 'created'
            ORDER BY id
            """
        ).fetchall()
        if any(row["trace_id"] == trace_id and row["name"] == path for row in rows):
            break
        time.sleep(0.25)
    matches = [row for row in rows if row["trace_id"] == trace_id and row["name"] == path]
    assert len(matches) == 1, [dict(row) for row in rows]
    row = matches[0]
    _assert_event_id(row["event_id"])
    assert row["path"] == path, dict(row)
    assert row["directory"] == ".", dict(row)
    assert row["size"] == len(file_content.encode()), dict(row)
    if credential_refs:
        assert row["credential_ref"] in credential_refs, dict(row)
    return row


def _assert_no_raw_secret_in_db(
    conn: sqlite3.Connection,
    raw_secrets: tuple[str, ...],
) -> None:
    if not raw_secrets:
        return
    tables = [
        row[0]
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
        ).fetchall()
    ]
    for table in tables:
        columns = conn.execute(f"PRAGMA table_info({table})").fetchall()
        text_columns = [row[1] for row in columns if str(row[2]).upper() in {"TEXT", ""}]
        if not text_columns:
            continue
        selected = ", ".join(f'"{column}"' for column in text_columns)
        for row in conn.execute(f'SELECT {selected} FROM "{table}"').fetchall():
            for column, value in zip(text_columns, row, strict=True):
                for raw_secret in raw_secrets:
                    assert raw_secret not in str(value), (
                        f"raw secret leaked in {table}.{column}"
                    )


def _assert_no_raw_secret_in_logs(log_paths: tuple[Path, ...], raw_secrets: tuple[str, ...]) -> None:
    for path in log_paths:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for raw_secret in raw_secrets:
            assert raw_secret not in text, f"raw secret leaked in {path}"
