"""AGY OAuth/broker ledger release gate."""

from __future__ import annotations

from contextlib import closing
from pathlib import Path
import re
import sqlite3

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_scripts import agy_cli_script
from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv

pytestmark = pytest.mark.integration


def test_agy_replay_records_google_credential_broker_events(
    model_client_env: ModelClientEnv,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        agy_cli_script(model_client_env.mock_base_url),
    )

    with closing(sqlite3.connect(f"file:{model_client_env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        net_rows = conn.execute(
            """
            SELECT credential_ref
            FROM net_events
            WHERE domain = 'daily-cloudcode-pa.googleapis.com'
              AND path = '/v1internal:streamGenerateContent'
            ORDER BY id
            """
        ).fetchall()
        assert net_rows, result
        credential_refs = {row["credential_ref"] for row in net_rows}
        assert len(credential_refs) == 1, [dict(row) for row in net_rows]
        credential_ref = next(iter(credential_refs))
        assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", credential_ref), credential_ref

        substitution_rows = conn.execute(
            """
            SELECT provider, source, outcome, algorithm, material_class, substitution_ref
            FROM substitution_events
            WHERE substitution_ref = ?
            ORDER BY id
            """,
            (credential_ref,),
        ).fetchall()

    assert substitution_rows, credential_ref
    assert {"captured", "brokered"} <= {row["outcome"] for row in substitution_rows}
    assert all(row["provider"] == "google" for row in substitution_rows)
    assert all(row["algorithm"] == "blake3" for row in substitution_rows)
    assert all(row["material_class"] == "credential" for row in substitution_rows)
    assert "http.header.authorization" in {
        row["source"] for row in substitution_rows if row["outcome"] == "captured"
    }


def test_agy_profile_root_does_not_bake_oauth_token_material() -> None:
    forbidden = (
        "antigravity-oauth-token",
        "access_token",
        "refresh_token",
        "id_token",
    )
    profile_root = Path("config/profiles/code/root")
    assert profile_root.exists()
    for path in profile_root.rglob("*"):
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        for needle in forbidden:
            assert needle not in text, f"{path} must not bake AGY OAuth token material"
