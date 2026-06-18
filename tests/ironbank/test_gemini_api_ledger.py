"""Ironbank black-box Gemini API ledger contract tests."""

from __future__ import annotations

from contextlib import closing
import sqlite3

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_scripts import gemini_api_script
from ironbank.model_pricing import assert_model_call_price
from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv


def test_gemini_api_streaming_and_nonstreaming_ledger_contract(
    model_client_env: ModelClientEnv,
):
    result = assert_one_model_client(
        model_client_env,
        gemini_api_script("https://generativelanguage.googleapis.com"),
    )
    assert result["provider"] == "google"
    assert result["credential_provider"] == "google"
    assert result["domain"] == "generativelanguage.googleapis.com"
    assert result["path"] == "/v1beta/models/gemini-3.5-flash:streamGenerateContent"
    assert result["model"] == "gemini-3.5-flash"
    assert result["nonstream_path"] == "/v1beta/models/gemini-3.5-flash:generateContent"
    assert result["nonstream_model"] == "gemini-3.5-flash"
    assert result["nonce"] in result["nonstream_text"]

    with closing(sqlite3.connect(f"file:{model_client_env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = 'google'
              AND path = ?
              AND model = ?
            ORDER BY id
            """,
            (result["nonstream_path"], result["nonstream_model"]),
        ).fetchall()
        assert len(rows) == 1, [dict(row) for row in rows]
        row = rows[0]
        assert row["method"] == "POST", dict(row)
        assert row["status_code"] == 200, dict(row)
        assert row["input_tokens"] == 11, dict(row)
        assert row["output_tokens"] == 7, dict(row)
        assert row["text_content"] == result["nonstream_text"], dict(row)
        assert row["credential_ref"], dict(row)
        assert row["request_bytes"] > 0, dict(row)
        assert row["response_bytes"] > 0, dict(row)
        assert_model_call_price(row)

        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = 'generativelanguage.googleapis.com'
              AND path = ?
            ORDER BY id
            """,
            (result["nonstream_path"],),
        ).fetchall()
        assert len(net_rows) == 1, [dict(row) for row in net_rows]
        net = net_rows[0]
        assert net["decision"] == "allowed", dict(net)
        assert net["credential_ref"] == row["credential_ref"], dict(net)
        assert "AIza" not in (net["request_headers"] or ""), dict(net)
        assert "hash:" in (net["request_headers"] or ""), dict(net)
