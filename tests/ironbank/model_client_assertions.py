"""Reusable assertions for Ironbank model client tests."""

from __future__ import annotations

from contextlib import closing
from pathlib import Path
from typing import Protocol

from helpers.body_archive import SessionArchive
from helpers.session_ledger import open_session_ledger
from ironbank.model_ledger import (
    ModelLedgerRun,
    ModelLedgerSpec,
    ModelLedgerTurn,
    TwoTurnModelLedgerSpec,
    assert_live_model_ledger_exchange,
    assert_model_ledger_exchange,
    assert_two_turn_model_ledger_exchange,
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
    # The decided-about event is archive-backed, so the search reads the
    # payloads rather than a column that no longer holds them.
    with closing(open_session_ledger(env.db_path)) as conn:
        event_ids = [
            row[0]
            for row in conn.execute(
                "SELECT event_id FROM security_decision_events WHERE event_type = 'file.import' "
                "ORDER BY id DESC"
            ).fetchall()
        ]
    # Searched as stored text, the way the LIKE over the column was, so
    # re-serializing the parsed JSON cannot change what matches.
    with SessionArchive(env.db_path) as archive:
        preserved = [
            event_id
            for event_id in event_ids
            if expected_text.encode()
            in (archive.read(event_id, "security_decision_events", "payload") or b"")
        ]
    assert preserved, f"imported script ledger should preserve {expected_text!r}"


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
        credential_source=result.get("credential_source"),
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
    return result


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
        credential_source=result.get("credential_source"),
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


def assert_two_turn_model_client(env: ModelClientEnvironment, script: str) -> dict:
    result = env.run_python(script)
    turns = tuple(
        ModelLedgerTurn(
            input=item["input"],
            reasoning=item["reasoning"],
            output=item["output"],
            tool_call_name=item["tool_call_name"],
            call_args=item["call_args"],
            call_response=item["call_response"],
            file_path=item["target"],
            file_content=item["nonce"] + "\n",
            call_id=item.get("call_id"),
        )
        for item in result["results"]
    )
    assert len(turns) == 2, result
    assert all(item["file_matches"] for item in result["results"]), result
    assert len({item["filename"] for item in result["results"]}) == 2, result
    spec = TwoTurnModelLedgerSpec(
        provider=result["provider"],
        domain=result["domain"],
        path=result["path"],
        model=result["model"],
        dns_qname=result["dns_qname"],
        dns_ip=result["dns_ip"],
        turns=turns,
        credential_provider=result.get("credential_provider") or result["provider"],
    )
    raw_secret = "sk-" + result["credential_nonce"]
    run = ModelLedgerRun(
        db_path=env.db_path,
        upstream_transcript_path=env.upstream_transcript_path,
        log_paths=env.log_paths,
        raw_secrets=(raw_secret,),
    )
    assert_two_turn_model_ledger_exchange(spec, run)
    return result


def _derive_model_client_raw_secrets(result: dict) -> tuple[str, ...]:
    provider = result.get("credential_provider") or result["provider"]
    if provider == "openai":
        return ("sk-" + result["nonce"],)
    if provider == "anthropic":
        return ("sk-ant-" + result["nonce"],)
    if provider == "google":
        return ("AIza" + result["nonce"],)
    return ()
