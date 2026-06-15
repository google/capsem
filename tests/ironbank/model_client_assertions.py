"""Reusable assertions for Ironbank model client tests."""

from __future__ import annotations

from contextlib import closing
from pathlib import Path
import sqlite3
from typing import Protocol

from ironbank.model_ledger import (
    ModelLedgerRun,
    ModelLedgerSpec,
    assert_live_model_ledger_exchange,
    assert_model_ledger_exchange,
)


class ModelClientEnvironment(Protocol):
    db_path: Path
    upstream_transcript_path: Path
    log_paths: tuple[Path, ...]

    def run_python(self, script: str, *, timeout_secs: int = 240) -> dict: ...


def assert_imported_script_contains(
    env: ModelClientEnvironment,
    expected_text: str,
) -> None:
    with closing(sqlite3.connect(f"file:{env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            """
            SELECT event_json
            FROM security_decision_events
            WHERE event_type = 'file.import'
              AND event_json LIKE ?
            ORDER BY id DESC
            """,
            (f"%{expected_text}%",),
        ).fetchall()
    assert rows, f"imported script ledger should preserve {expected_text!r}"


def assert_one_model_client(
    env: ModelClientEnvironment,
    script: str,
    *,
    raw_secrets: tuple[str, ...] = (),
    expected_imported_text: str | None = None,
) -> None:
    result = env.run_python(script)
    assert result["file_matches"] is True, result
    derived_raw_secrets = raw_secrets or _derive_model_client_raw_secrets(result)
    spec = ModelLedgerSpec(
        input=result["input"],
        reasoning=result["reasoning"],
        output=result["output"],
        tool_call_name=result["tool_call_name"],
        call_args=result["call_args"],
        call_response=result["call_response"],
        provider=result["provider"],
        domain=result["domain"],
        path=result["path"],
        model=result["model"],
        credential_provider=result.get("credential_provider"),
    )
    run = ModelLedgerRun(
        db_path=env.db_path,
        upstream_transcript_path=env.upstream_transcript_path,
        log_paths=env.log_paths,
        raw_secrets=derived_raw_secrets,
    )
    assert_model_ledger_exchange(spec, run)
    if expected_imported_text is not None:
        assert_imported_script_contains(env, expected_imported_text)


def assert_live_model_client(
    env: ModelClientEnvironment,
    script: str,
    *,
    raw_secret: str,
    expected_credential_ref: str,
    expected_model_calls: int = 2,
    timeout_secs: int = 240,
) -> dict:
    result = env.run_python(script, timeout_secs=timeout_secs)
    assert result["file_matches"] is True, result
    if "output_contains_nonce" in result:
        assert result["output_contains_nonce"] is True, result
    spec = ModelLedgerSpec(
        input=result["input"],
        reasoning=result["reasoning"],
        output=result["output"],
        tool_call_name=result["tool_call_name"],
        call_args=result["call_args"],
        call_response=result["call_response"],
        provider=result["provider"],
        domain=result["domain"],
        path=result["path"],
        model=result["model"],
        credential_provider=result.get("credential_provider"),
    )
    run = ModelLedgerRun(
        db_path=env.db_path,
        upstream_transcript_path=env.upstream_transcript_path,
        log_paths=env.log_paths,
        raw_secrets=(raw_secret,),
        expected_credential_ref=expected_credential_ref,
    )
    assert_live_model_ledger_exchange(
        spec,
        run,
        expected_model_calls=expected_model_calls,
    )
    return result


def _derive_model_client_raw_secrets(result: dict) -> tuple[str, ...]:
    provider = result.get("credential_provider") or result["provider"]
    if provider == "openai":
        return ("sk-" + result["nonce"],)
    if provider == "anthropic":
        return ("sk-ant-" + result["nonce"],)
    return ()
