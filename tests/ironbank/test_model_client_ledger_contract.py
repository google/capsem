"""Ironbank model client ledger contract tests.

Each test owns one client surface and one deterministic tool-use exchange.
The shared assertion reconciles the client result, upstream transcript,
session DB, security ledger, files, and logs.
"""

from __future__ import annotations

from contextlib import closing
from dataclasses import dataclass
import json
import os
from pathlib import Path
import sqlite3
import textwrap
import time
import uuid

import blake3
import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import MOCK_SERVER_BINARY, start_mock_server, stop_process
from helpers.service import ServiceInstance, wait_exec_ready, vm_name
from ironbank.model_client_assertions import assert_live_model_client, assert_one_model_client
from ironbank.model_client_config import HERMETIC_OPENAI_PRICED_MODEL
from ironbank.model_ledger import _assert_event_id
from ironbank.model_pricing import assert_model_call_price
from ironbank.model_client_scripts import (
    agy_cli_script,
    claude_api_script,
    claude_ollama_launch_script,
    claude_sdk_script,
    claude_streaming_api_script,
    codex_cli_script,
    codex_ollama_launch_script,
    live_openai_chat_completions_script,
    live_openai_responses_api_script,
    openai_embeddings_and_image_script,
    openai_responses_api_script,
    openai_two_tool_calls_script,
)

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = PROJECT_ROOT / "assets"
PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"


def _eventually(query, predicate, *, timeout_s: float = 10.0, interval_s: float = 0.25):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = query()
        if predicate(last):
            return last
        time.sleep(interval_s)
    last = query()
    assert predicate(last), last
    return last


def _credential_ref(value: object) -> str:
    import re

    assert isinstance(value, str)
    assert re.fullmatch(r"credential:blake3:[0-9a-f]{64}", value), value
    return value


def _assert_raw_absent_from_db(conn, raw_secret: str) -> None:
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
                assert raw_secret not in str(value), f"raw secret leaked in {table}.{column}"


def _live_provider_secret(name: str) -> str | None:
    value = os.environ.get(name)
    if value:
        return value
    candidates: list[Path] = []
    if os.environ.get("CAPSEM_LIVE_PROVIDER_DOTENV"):
        candidates.append(Path(os.environ["CAPSEM_LIVE_PROVIDER_DOTENV"]))
    candidates.append(PROJECT_ROOT / ".env")
    for dotenv in candidates:
        if not dotenv.exists():
            continue
        for line in dotenv.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or "=" not in stripped:
                continue
            key, raw_value = stripped.split("=", 1)
            if key == name:
                return raw_value.strip().strip('"').strip("'")
    return None


def _credential_ref_for_secret(secret: str, *, provider: str = "openai") -> str:
    hasher = blake3.blake3()
    hasher.update(b"capsem.credential.v1")
    hasher.update(b"\0")
    hasher.update(provider.encode("utf-8"))
    hasher.update(b"\0")
    hasher.update(secret.encode("utf-8"))
    return f"credential:blake3:{hasher.hexdigest()}"


@dataclass
class ModelClientEnv:
    service: ServiceInstance
    client: object
    session_id: str
    mock_base_url: str
    upstream_transcript_path: Path

    @property
    def db_path(self) -> Path:
        return self.service.tmp_dir / "sessions" / self.session_id / "session.db"

    @property
    def log_paths(self) -> tuple[Path, ...]:
        session_dir = self.service.tmp_dir / "sessions" / self.session_id
        return (
            self.service.tmp_dir / "service.log",
            self.service.tmp_dir / "service.stderr.log",
            session_dir / "process.log",
            session_dir / "serial.log",
        )

    def run_python(self, script: str, *, timeout_secs: int = 240) -> dict:
        script_name = f"ironbank-client-{uuid.uuid4().hex[:8]}.py"
        payload = script.encode()
        upload = self.client.post_bytes(
            f"/vms/{self.session_id}/files/content?path={script_name}",
            payload,
            timeout=30,
        )
        assert upload is not None
        assert upload["success"] is True
        assert upload["size"] == len(payload)
        exec_resp = self.client.post(
            f"/vms/{self.session_id}/exec",
            {"command": f"python3 /root/{script_name}", "timeout_secs": timeout_secs},
            timeout=timeout_secs + 30,
        )
        assert exec_resp is not None
        assert exec_resp["exit_code"] == 0, exec_resp
        stdout = exec_resp.get("stdout") or ""
        stderr = exec_resp.get("stderr") or ""
        line = next(
            (line for line in stdout.splitlines() if line.startswith("IRONBANK_CLIENT_RESULT=")),
            None,
        )
        assert line is not None, stdout + stderr
        return json.loads(line.split("=", 1)[1])


@pytest.fixture
def model_client_env():
    assert MOCK_SERVER_BINARY.exists(), f"{MOCK_SERVER_BINARY} missing"
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before Ironbank"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    client = None
    mock_proc = None
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    session_id = vm_name("ironbank-model")
    try:
        mock_proc, ready = start_mock_server(
            request_log=service.tmp_dir / "upstream-transcript.jsonl"
        )
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                f"""
                refresh_policy = "24h"

                [network.dns]
                upstreams = [{json.dumps(ready["dns_udp_addr"])}]

                [network.upstream_overrides."daily-cloudcode-pa.googleapis.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [network.upstream_overrides."www.googleapis.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [network.upstream_overrides."play.googleapis.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [network.upstream_overrides."antigravity-unleash.goog:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [network.upstream_overrides."api.openai.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [network.upstream_overrides."api.anthropic.com:443"]
                dial = {json.dumps(ready["http_addr"])}
                protocol = "http"

                [settings."security.web.http_upstream_ports"]
                value = [80, 3713, 8080, 11434]
                modified = "2026-06-14T00:00:00Z"

                [ai.ollama]
                name = "Ollama"
                protocol = "ollama"
                url = "http://127.0.0.1:3713"
                listen_ports = [3713]
                allowed_remote_targets = ["127.0.0.1:3713"]

                [ai.ollama.rules.local_fixture_endpoint]
                name = "ollama_local_fixture_endpoint"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Declare the hermetic Ollama-compatible endpoint for Ironbank launcher tests."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && (http.path == "/" || http.path == "/api/show" || http.path == "/api/tags" || http.path == "/api/chat" || http.path == "/v1/responses" || http.path == "/v1/messages")'

                [corp.rules.allow_ironbank_mock_model_server]
                name = "allow_ironbank_mock_model_server"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow the hermetic Ironbank model fixture while preserving local-network ask defaults."
                match = 'http.host == "127.0.0.1" && tcp.port == "3713" && (http.path == "/" || http.path == "/api/show" || http.path == "/api/tags" || http.path == "/api/chat" || http.path == "/v1/responses" || http.path == "/v1/messages")'

                [corp.rules.allow_ironbank_google_code_assist]
                name = "allow_ironbank_google_code_assist"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow hermetic AGY Google Code Assist replay through the declared upstream override."
                match = 'tcp.port == "443" && ((http.host == "daily-cloudcode-pa.googleapis.com" && http.path.matches("^/v1internal:")) || (http.host == "www.googleapis.com" && http.path == "/oauth2/v2/userinfo") || (http.host == "play.googleapis.com" && http.path == "/log") || (http.host == "antigravity-unleash.goog" && http.path.matches("^/api/client/")))'

                [corp.rules.allow_ironbank_openai_api]
                name = "allow_ironbank_openai_api"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow hermetic OpenAI API replay through the declared upstream override."
                match = 'tcp.port == "443" && http.host == "api.openai.com" && http.path.matches("^/v1/")'

                [corp.rules.allow_ironbank_anthropic_api]
                name = "allow_ironbank_anthropic_api"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow hermetic Anthropic API replay through the declared upstream override."
                match = 'tcp.port == "443" && http.host == "api.anthropic.com" && http.path.matches("^/v1/")'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": {"CAPSEM_MOCK_SERVER_BASE_URL": ready["base_url"]},
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        active_profile = service.tmp_dir / "sessions" / session_id / "vm" / "active_profile.toml"
        assert active_profile.exists(), f"active profile missing at {active_profile}"
        active_profile_text = active_profile.read_text(encoding="utf-8")
        assert ready["dns_udp_addr"] in active_profile_text
        assert ready["http_addr"] in active_profile_text
        assert "api.openai.com:443" in active_profile_text
        assert "api.anthropic.com:443" in active_profile_text
        assert "daily-cloudcode-pa.googleapis.com:443" in active_profile_text
        assert "antigravity-unleash.goog:443" in active_profile_text
        assert "runtime-overlay.toml" not in active_profile_text
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)
        yield ModelClientEnv(
            service=service,
            client=client,
            session_id=session_id,
            mock_base_url=ready["base_url"],
            upstream_transcript_path=Path(ready["request_log"]),
        )
    finally:
        stop_process(mock_proc)
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config


@pytest.fixture
def live_model_client_env():
    assert ASSETS_DIR.exists(), f"{ASSETS_DIR} missing; build VM assets before live canary"
    assert PROFILES_DIR.exists(), f"{PROFILES_DIR} missing; materialize profile config"

    service = ServiceInstance()
    client = None
    old_corp_config = os.environ.get("CAPSEM_CORP_CONFIG")
    session_id = vm_name("ironbank-live-model")
    vm_env = {
        key: value
        for key in ("OPENAI_API_KEY", "GOOGLE_API_KEY", "GEMINI_API_KEY")
        if (value := _live_provider_secret(key))
    }
    try:
        corp_path = service.tmp_dir / "corp.toml"
        corp_path.write_text(
            textwrap.dedent(
                """
                refresh_policy = "24h"

                [corp.rules.allow_live_provider_canary]
                name = "allow_live_provider_canary"
                action = "allow"
                priority = -100
                detection_level = "informational"
                reason = "Allow optional live-provider compatibility canaries when an operator explicitly provides credentials."
                match = 'http.host.matches("(^|.*\\.)(openai\\.com|googleapis\\.com)$")'
                """
            ).strip()
            + "\n",
            encoding="utf-8",
        )
        os.environ["CAPSEM_CORP_CONFIG"] = str(corp_path)
        service.start()
        client = service.client()
        create = client.post(
            "/vms/create",
            {
                "name": session_id,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
                "env": vm_env,
            },
            timeout=90,
        )
        assert create is not None
        assert create.get("id") == session_id or create.get("name") == session_id
        assert wait_exec_ready(client, session_id, timeout=EXEC_READY_TIMEOUT)
        transcript_path = service.tmp_dir / "live-provider-transcript-unused.jsonl"
        transcript_path.write_text("", encoding="utf-8")
        yield ModelClientEnv(
            service=service,
            client=client,
            session_id=session_id,
            mock_base_url="https://live-provider.invalid",
            upstream_transcript_path=transcript_path,
        )
    finally:
        if client is not None:
            try:
                client.delete(f"/vms/{session_id}/delete", timeout=60)
            except Exception:
                pass
        service.stop()
        if old_corp_config is None:
            os.environ.pop("CAPSEM_CORP_CONFIG", None)
        else:
            os.environ["CAPSEM_CORP_CONFIG"] = old_corp_config


def test_openai_responses_api_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(model_client_env, openai_responses_api_script("https://api.openai.com"))
    _assert_openai_embeddings_and_image_ledger(model_client_env)


def _assert_openai_embeddings_and_image_ledger(model_client_env: ModelClientEnv) -> None:
    result = model_client_env.run_python(
        openai_embeddings_and_image_script("https://api.openai.com")
    )
    raw_secret = "sk-" + result["credential_nonce"]
    expected_credential_ref = _credential_ref_for_secret(raw_secret)
    assert result["provider"] == "openai"
    assert result["domain"] == "api.openai.com"
    assert result["embedding_path"] == "/v1/embeddings"
    assert result["embedding_model"] == "text-embedding-3-small"
    assert result["embedding_vector"] == [0.125, -0.25, 0.5, 0.75]
    assert result["image_path"] == "/v1/images/generations"
    assert result["image_model"] == "gpt-5-image-mini"
    assert result["image_b64"] == "Y2Fwc2VtLW1vY2staW1hZ2U="

    with closing(sqlite3.connect(f"file:{model_client_env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        upstream_records = [
            json.loads(line)
            for line in model_client_env.upstream_transcript_path.read_text(
                encoding="utf-8"
            ).splitlines()
            if line.strip()
        ]
        embedding_upstream = [
            row for row in upstream_records if row.get("path") == result["embedding_path"]
        ]
        image_upstream = [
            row for row in upstream_records if row.get("path") == result["image_path"]
        ]
        assert len(embedding_upstream) == 1, embedding_upstream
        assert len(image_upstream) == 1, image_upstream
        assert result["embedding_input"] in embedding_upstream[0]["request_body"]
        assert result["embedding_model"] in embedding_upstream[0]["request_body"]
        assert result["image_prompt"] in image_upstream[0]["request_body"]
        assert result["image_model"] in image_upstream[0]["request_body"]
        assert result["image_b64"] in image_upstream[0]["response_body"]

        model_rows = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = 'openai'
              AND path IN ('/v1/embeddings', '/v1/images/generations')
            ORDER BY id
            """
        ).fetchall()
        by_path = {row["path"]: row for row in model_rows}
        assert set(by_path) == {"/v1/embeddings", "/v1/images/generations"}, [
            dict(row) for row in model_rows
        ]
        embedding_model = by_path["/v1/embeddings"]
        image_model = by_path["/v1/images/generations"]
        for row in (embedding_model, image_model):
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST", dict(row)
            assert row["status_code"] == 200, dict(row)
            assert row["request_bytes"] > 0, dict(row)
            assert row["response_bytes"] > 0, dict(row)
            assert row["credential_ref"] == expected_credential_ref, dict(row)
            assert row["trace_id"], dict(row)
            assert_model_call_price(row)
        assert embedding_model["model"] == result["embedding_model"], dict(embedding_model)
        assert embedding_model["input_tokens"] == 9, dict(embedding_model)
        assert embedding_model["output_tokens"] in {0, None}, dict(embedding_model)
        assert result["embedding_input"] in (
            embedding_model["request_body_preview"] or ""
        ), dict(embedding_model)
        assert image_model["model"] == result["image_model"], dict(image_model)
        assert image_model["input_tokens"] == 11, dict(image_model)
        assert image_model["output_tokens"] == 17, dict(image_model)
        assert result["image_prompt"] in (image_model["request_body_preview"] or ""), dict(
            image_model
        )
        assert result["image_b64"] in (image_model["text_content"] or ""), dict(
            image_model
        )

        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = 'api.openai.com'
              AND path IN ('/v1/embeddings', '/v1/images/generations')
            ORDER BY id
            """
        ).fetchall()
        net_by_path = {row["path"]: row for row in net_rows}
        assert set(net_by_path) == {"/v1/embeddings", "/v1/images/generations"}, [
            dict(row) for row in net_rows
        ]
        for path, row in net_by_path.items():
            _assert_event_id(row["event_id"])
            assert row["method"] == "POST", dict(row)
            assert row["status_code"] == 200, dict(row)
            assert row["decision"] == "allowed", dict(row)
            assert row["credential_ref"] == expected_credential_ref, dict(row)
            assert row["bytes_sent"] > 0, dict(row)
            assert row["bytes_received"] > 0, dict(row)
            request_headers = (row["request_headers"] or "").lower()
            assert "authorization: hash:" in request_headers, dict(row)
            assert raw_secret.lower() not in request_headers, dict(row)
            assert f"bearer {raw_secret.lower()}" not in request_headers, dict(row)
            if path == "/v1/embeddings":
                assert result["embedding_input"] in (
                    row["request_body_preview"] or ""
                ), dict(row)
            else:
                assert result["image_prompt"] in (row["request_body_preview"] or ""), dict(
                    row
                )
                assert result["image_b64"] in (row["response_body_preview"] or ""), dict(
                    row
                )

        event_ids = [row["event_id"] for row in (*model_rows, *net_rows)]
        placeholders = ",".join("?" for _ in event_ids)
        security_rows = conn.execute(
            f"""
            SELECT *
            FROM security_rule_events
            WHERE event_id IN ({placeholders})
            ORDER BY id
            """,
            event_ids,
        ).fetchall()
        assert {row["event_id"] for row in security_rows} >= set(event_ids), {
            "event_ids": event_ids,
            "security_rows": [dict(row) for row in security_rows],
        }
        assert all(json.loads(row["event_json"]) for row in security_rows)
        assert all(json.loads(row["rule_json"]) for row in security_rows)

        substitution_rows = conn.execute(
            """
            SELECT *
            FROM substitution_events
            WHERE substitution_ref = ?
            ORDER BY id
            """,
            (expected_credential_ref,),
        ).fetchall()
        assert {"captured", "brokered"} <= {row["outcome"] for row in substitution_rows}, [
            dict(row) for row in substitution_rows
        ]
        assert all(row["provider"] == "openai" for row in substitution_rows)
        _assert_raw_absent_from_db(conn, raw_secret)
    for log_path in model_client_env.log_paths:
        if log_path.exists():
            assert raw_secret not in log_path.read_text(
                encoding="utf-8", errors="replace"
            ), f"raw secret leaked in {log_path}"


@pytest.mark.live_provider
def test_live_openai_chat_completions_ledger_canary(
    live_model_client_env: ModelClientEnv,
):
    openai_key = _live_provider_secret("OPENAI_API_KEY")
    if not openai_key:
        pytest.skip("OPENAI_API_KEY not provided for optional live-provider canary")
    result = assert_live_model_client(
        live_model_client_env,
        live_openai_chat_completions_script(),
        raw_secret=openai_key,
        expected_credential_ref=_credential_ref_for_secret(openai_key),
        expected_model_calls=2,
        timeout_secs=240,
    )
    assert result["provider"] == "openai"
    assert result["domain"] == "api.openai.com"
    assert result["path"] == "/v1/chat/completions"


@pytest.mark.live_provider
def test_live_openai_responses_api_ledger_canary(live_model_client_env: ModelClientEnv):
    openai_key = _live_provider_secret("OPENAI_API_KEY")
    if not openai_key:
        pytest.skip("OPENAI_API_KEY not provided for optional live-provider canary")
    result = assert_live_model_client(
        live_model_client_env,
        live_openai_responses_api_script(),
        raw_secret=openai_key,
        expected_credential_ref=_credential_ref_for_secret(openai_key),
        expected_model_calls=2,
        timeout_secs=240,
    )
    assert result["provider"] == "openai"
    assert result["domain"] == "api.openai.com"
    assert result["path"] == "/v1/responses"


def test_openai_two_tool_calls_have_exact_item_cardinality(
    model_client_env: ModelClientEnv,
):
    result = model_client_env.run_python(openai_two_tool_calls_script("https://api.openai.com"))
    assert len(result["results"]) == 2, result
    assert all(item["file_matches"] for item in result["results"]), result
    assert len({item["call_id"] for item in result["results"]}) == 2, result
    assert len({item["filename"] for item in result["results"]}) == 2, result
    raw_secret = "sk-" + result["credential_nonce"]

    import sqlite3

    with closing(sqlite3.connect(f"file:{model_client_env.db_path}?mode=ro", uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        tables = {
            row[0]
            for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        assert "model_items" in tables, (
            "RED: OpenAI two-tool-call ledger needs first-class model_items rows "
            "with per-trace exact cardinality: one request, one reasoning, "
            "one response, one tool_call, one tool_response, and one created file"
        )
        model_calls = conn.execute(
            """
            SELECT *
            FROM model_calls
            WHERE provider = 'openai'
              AND path = '/v1/responses'
              AND model = ?
            ORDER BY id
            """,
            (HERMETIC_OPENAI_PRICED_MODEL,),
        ).fetchall()
        assert len(model_calls) == 4, [dict(row) for row in model_calls]
        assert {row["method"] for row in model_calls} == {"POST"}
        assert {row["status_code"] for row in model_calls} == {200}
        assert all(row["request_bytes"] > 0 for row in model_calls)
        assert all(row["response_bytes"] > 0 for row in model_calls)
        for row in model_calls:
            assert_model_call_price(row)

        item_rows = conn.execute(
            """
            SELECT *
            FROM model_items
            WHERE provider = 'openai'
              AND path = '/v1/responses'
              AND model = ?
            ORDER BY id
            """,
            (HERMETIC_OPENAI_PRICED_MODEL,),
        ).fetchall()
        by_trace: dict[str, list[sqlite3.Row]] = {}
        for row in item_rows:
            by_trace.setdefault(row["trace_id"], []).append(row)
        assert len(by_trace) == 2, [dict(row) for row in item_rows]
        assert len(item_rows) == 10, [dict(row) for row in item_rows]
        assert all(row["provider"] == "openai" for row in item_rows)
        assert all(row["path"] == "/v1/responses" for row in item_rows)
        assert all(row["model"] == HERMETIC_OPENAI_PRICED_MODEL for row in item_rows)
        assert all(
            isinstance(row["content_hash"], str)
            and len(row["content_hash"]) == 71
            and row["content_hash"].startswith("blake3:")
            for row in item_rows
        )

        tool_calls = conn.execute(
            "SELECT * FROM tool_calls WHERE tool_name = 'exec_command' ORDER BY id"
        ).fetchall()
        tool_responses = conn.execute("SELECT * FROM tool_responses ORDER BY id").fetchall()
        expected_filenames = {item["filename"] for item in result["results"]}
        file_rows = _eventually(
            lambda: conn.execute(
                """
                SELECT *
                FROM fs_events
                WHERE action = 'created'
                ORDER BY id
                """
            ).fetchall(),
            lambda rows: expected_filenames
            <= {row["name"] for row in rows if row["name"] is not None},
            timeout_s=15,
        )
        net_rows = conn.execute(
            """
            SELECT *
            FROM net_events
            WHERE domain = 'api.openai.com'
              AND path = '/v1/responses'
            ORDER BY id
            """
        ).fetchall()
        assert len(net_rows) == 4, [dict(row) for row in net_rows]
        assert all(row["method"] == "POST" for row in net_rows)
        assert all(row["status_code"] == 200 for row in net_rows)
        assert all(row["decision"] == "allowed" for row in net_rows)
        assert all(row["bytes_sent"] > 0 for row in net_rows)
        assert all(row["bytes_received"] > 0 for row in net_rows)
        credential_refs = {_credential_ref(row["credential_ref"]) for row in net_rows}
        assert len(credential_refs) == 1, [dict(row) for row in net_rows]
        credential_ref = next(iter(credential_refs))
        assert {row["credential_ref"] for row in model_calls} == {credential_ref}, [
            dict(row) for row in model_calls
        ]
        assert {row["credential_ref"] for row in tool_calls} == {credential_ref}, [
            dict(row) for row in tool_calls
        ]
        assert {row["credential_ref"] for row in tool_responses} == {credential_ref}, [
            dict(row) for row in tool_responses
        ]
        substitution_rows = conn.execute(
            """
            SELECT *
            FROM substitution_events
            WHERE substitution_ref = ?
            ORDER BY id
            """,
            (credential_ref,),
        ).fetchall()
        assert substitution_rows, credential_ref
        assert {"captured", "brokered"} <= {row["outcome"] for row in substitution_rows}, [
            dict(row) for row in substitution_rows
        ]
        assert all(row["provider"] == "openai" for row in substitution_rows)
        assert all(row["algorithm"] == "blake3" for row in substitution_rows)
        assert all(row["material_class"] == "credential" for row in substitution_rows)
        assert "http.header.authorization" in {
            row["source"] for row in substitution_rows if row["outcome"] == "captured"
        }
        _assert_raw_absent_from_db(conn, raw_secret)

        dns_rows = conn.execute(
            """
            SELECT *
            FROM dns_events
            WHERE qname = ?
            ORDER BY id
            """,
            (result["dns_qname"],),
        ).fetchall()
        assert len(dns_rows) == 1, [dict(row) for row in dns_rows]
        dns = dns_rows[0]
        assert dns["qtype"] == 1, dict(dns)
        assert dns["qclass"] == 1, dict(dns)
        assert dns["rcode"] == 0, dict(dns)
        assert dns["decision"] == "allowed", dict(dns)
        assert dns["answer_ip"] == result["dns_ip"] == "127.0.0.1", dict(dns)
        assert dns["source_proto"] in {"udp", "tcp"}, dict(dns)

        file_event_ids = []
        for expected in result["results"]:
            trace_matches = [
                trace_id
                for trace_id, rows in by_trace.items()
                if any(expected["input"] in (row["content"] or "") for row in rows)
                or any(expected["output"] in (row["content"] or "") for row in rows)
            ]
            assert len(trace_matches) == 1, {
                "expected": expected,
                "model_items": [dict(row) for row in item_rows],
            }
            trace_id = trace_matches[0]
            rows = by_trace[trace_id]
            trace_model_calls = [row for row in model_calls if row["trace_id"] == trace_id]
            assert len(trace_model_calls) == 2, [dict(row) for row in model_calls]
            trace_net_rows = [row for row in net_rows if row["trace_id"] == trace_id]
            assert len(trace_net_rows) == 2, [dict(row) for row in net_rows]

            assert sum(row["kind"] == "request" for row in rows) == 1
            assert sum(row["kind"] == "reasoning" for row in rows) == 1
            assert sum(row["kind"] == "response" for row in rows) == 1
            assert sum(row["kind"] == "tool_call" for row in rows) == 1
            assert sum(row["kind"] == "tool_response" for row in rows) == 1
            request_row = next(row for row in rows if row["kind"] == "request")
            reasoning_row = next(row for row in rows if row["kind"] == "reasoning")
            response_row = next(row for row in rows if row["kind"] == "response")
            tool_call_row = next(row for row in rows if row["kind"] == "tool_call")
            tool_response_row = next(row for row in rows if row["kind"] == "tool_response")

            assert expected["input"] in (request_row["content"] or "")
            assert expected["target"] in (request_row["content"] or "")
            assert '"tools"' in (request_row["content"] or "")
            assert "exec_command" in (request_row["content"] or "")
            assert reasoning_row["content"] == expected["reasoning"]
            assert response_row["content"] == expected["output"]
            assert tool_call_row["call_id"] == expected["call_id"]
            assert tool_call_row["tool_name"] == expected["tool_call_name"]
            assert json.loads(tool_call_row["arguments"]) == expected["call_args"]
            assert expected["target"] in (tool_call_row["content"] or "")
            assert expected["nonce"] in (tool_call_row["content"] or "")
            assert tool_response_row["call_id"] == expected["call_id"]
            assert tool_response_row["content"] == expected["call_response"]

            trace_tool_calls = [row for row in tool_calls if row["trace_id"] == trace_id]
            assert len(trace_tool_calls) == 1, [dict(row) for row in tool_calls]
            assert trace_tool_calls[0]["call_id"] == expected["call_id"]
            assert json.loads(trace_tool_calls[0]["arguments"]) == expected["call_args"]
            trace_tool_responses = [
                row for row in tool_responses if row["trace_id"] == trace_id
            ]
            assert len(trace_tool_responses) == 1, [dict(row) for row in tool_responses]
            assert trace_tool_responses[0]["call_id"] == expected["call_id"]
            assert expected["call_response"] in (
                trace_tool_responses[0]["content_preview"] or ""
            )
            created = [
                row
                for row in file_rows
                if row["trace_id"] == trace_id and row["name"] == expected["filename"]
            ]
            assert len(created) == 1, [dict(row) for row in file_rows]
            assert created[0]["size"] == len((expected["nonce"] + "\n").encode())
            assert created[0]["directory"] == ".", dict(created[0])
            assert created[0]["credential_ref"] == credential_ref, dict(created[0])
            file_event_ids.append(created[0]["event_id"])

        event_ids = [row["event_id"] for row in [*model_calls, *net_rows, dns]]
        event_ids.extend(file_event_ids)
        placeholders = ",".join("?" for _ in event_ids)
        rule_rows = conn.execute(
            f"""
            SELECT *
            FROM security_rule_events
            WHERE event_id IN ({placeholders})
            ORDER BY id
            """,
            event_ids,
        ).fetchall()
        assert rule_rows, event_ids
        covered = {row["event_id"] for row in rule_rows}
        assert set(event_ids) <= covered, {
            "missing": sorted(set(event_ids) - covered),
            "rows": [dict(row) for row in rule_rows],
        }
        assert all(
            row["rule_action"]
            in {"allow", "ask", "block", "preprocess", "rewrite", "postprocess"}
            for row in rule_rows
        )
        assert all(
            row["detection_level"]
            in {"none", "informational", "low", "medium", "high", "critical"}
            for row in rule_rows
        )
        assert all(json.loads(row["event_json"]) for row in rule_rows)
        assert all(json.loads(row["rule_json"]) for row in rule_rows)
    for log_path in model_client_env.log_paths:
        if log_path.exists():
            assert raw_secret not in log_path.read_text(
                encoding="utf-8", errors="replace"
            ), f"raw secret leaked in {log_path}"


def test_codex_cli_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        codex_cli_script(model_client_env.mock_base_url),
    )


def test_claude_http_api_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        claude_api_script("https://api.anthropic.com"),
    )
    model_client_env.upstream_transcript_path.write_text("", encoding="utf-8")
    assert_one_model_client(
        model_client_env,
        claude_streaming_api_script("https://api.anthropic.com"),
    )


def test_claude_sdk_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        claude_sdk_script("https://api.anthropic.com"),
    )


def test_ollama_launch_claude_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        claude_ollama_launch_script(model_client_env.mock_base_url),
    )


def test_ollama_launch_codex_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(
        model_client_env,
        codex_ollama_launch_script(model_client_env.mock_base_url),
    )


def test_agy_cli_ledger_contract(model_client_env: ModelClientEnv):
    assert_one_model_client(model_client_env, agy_cli_script(model_client_env.mock_base_url))
