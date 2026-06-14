"""Black-box model ledger checks for Ironbank tests."""

from __future__ import annotations

from contextlib import closing
from dataclasses import dataclass
import json
import re
import sqlite3
from pathlib import Path
from typing import Any


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


@dataclass(frozen=True)
class ModelLedgerRun:
    db_path: Path
    upstream_transcript_path: Path
    log_paths: tuple[Path, ...]
    raw_secrets: tuple[str, ...] = ()


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
        command = spec.call_args.get("cmd") or spec.call_args.get("command")
        if isinstance(command, str):
            assert Path(command.rsplit(">", 1)[-1].strip()).name in upstream_outputs
        assert spec.call_response in upstream_inputs

        expected_usage = [_usage_from_upstream(row) for row in upstream_records]
        expected_usage = [usage for usage in expected_usage if usage is not None]
        assert expected_usage, f"upstream transcript lacks usage for {spec.path}"

        model_rows = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = ? AND path = ? AND model = ?
            ORDER BY id
            """,
            (spec.provider, spec.path, spec.model),
        ).fetchall()
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
            ORDER BY tool_calls.id
            """,
            (spec.provider, spec.tool_call_name, spec.path, spec.model),
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
            ORDER BY id
            """,
            (tool_row["call_id"],),
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
        for row in net_rows:
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST"
            assert row["status_code"] == 200
            assert row["decision"] == "allowed"
            assert row["bytes_sent"] > 0
            assert row["bytes_received"] > 0
            assert spec.input in (row["request_body_preview"] or "")
            assert spec.tool_call_name in (row["response_body_preview"] or "")

        _assert_security_rows(conn, [row["event_id"] for row in (*model_rows, *net_rows)])
        _assert_tool_output_file(conn, spec)
        _assert_no_raw_secret_in_db(conn, run.raw_secrets)
    _assert_no_raw_secret_in_logs(run.log_paths, run.raw_secrets)


def _connect(db_path: Path) -> sqlite3.Connection:
    assert db_path.exists(), f"missing session DB: {db_path}"
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def _load_upstream_records(path: Path, model_path: str) -> list[dict[str, Any]]:
    assert path.exists(), f"missing upstream transcript: {path}"
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and json.loads(line)["path"] == model_path
    ]


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
        payload = response_payloads[-1] if response_payloads else {}
    else:
        payload = json.loads(body)

    usage = payload.get("usage")
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


def _assert_tool_output_file(conn: sqlite3.Connection, spec: ModelLedgerSpec) -> None:
    command = spec.call_args.get("cmd") or spec.call_args.get("command")
    if not isinstance(command, str):
        return
    match = re.search(r">\s*(/root/[^ ]+)", command)
    if not match:
        return
    path = Path(match.group(1)).name
    rows = conn.execute(
        """
        SELECT *
        FROM fs_events
        WHERE name = ? OR path = ?
        ORDER BY id
        """,
        (path, path),
    ).fetchall()
    assert rows, f"missing fs_events for tool output {path}"
    assert any(row["action"] in {"created", "modified", "export"} for row in rows)
    assert all(row["name"] in {path, None} for row in rows)


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
