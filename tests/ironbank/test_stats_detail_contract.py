"""Ironbank stats/detail route contract.

The desktop stats UI must be a projection of session.db and public routes, not
invented preview fields or duplicated payload renderings. This test serves a
real ledger -- the checked-in session fixture, which the logger keeps in the
writer's exact shape, with its body archive -- plus rows for the layers that
fixture never recorded, and holds each route to what the ledger itself says.

It used to hand-write the schema and put bodies in a column. The ledger moved
bodies to the block archive and grew rule runs and a transport marker, and
that copy fell behind three times; the service now refuses such a file
outright. The ledger here is one the writer wrote.
"""

from __future__ import annotations

import base64
import json
import os
import platform
import shutil
import sqlite3
import tomllib
from contextlib import closing
from pathlib import Path

import pytest
from helpers.body_archive import archived_bodies
from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import ServiceInstance, materialize_test_profiles
from helpers.session_ledger import ledger_counters, open_session_ledger

pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DB = PROJECT_ROOT / "tests/fixtures/session/test.db"

SESSION_ID = "code-stats-ledger"
TRACE_ID = "trace-stats-ledger"
DNS_EVENT_ID = "d1e2f3a4b5c6"
FILE_EVENT_ID = "e1f2a3b4c5d6"
EXEC_EVENT_ID = "f1a2b3c4d5e6"
CRED_EVENT_ID = "abc123def456"
SEC_EVENT_ID = "123abc456def"
CREDENTIAL_REF = "credential:blake3:" + "1" * 64
#: Every `*_preview` column is a display excerpt of at most this many bytes.
PREVIEW_BYTES = 2048


def _profile_contract(tmp_dir: Path) -> dict[str, object]:
    profiles_dir = materialize_test_profiles(tmp_dir)
    profile = tomllib.loads((profiles_dir / CODE_PROFILE_ID / "profile.toml").read_text())
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    assets = profile["assets"]["arch"][arch]
    return {
        "revision": profile["revision"],
        "pins": {
            "kernel": {"name": assets["kernel"]["name"], "hash": assets["kernel"]["hash"]},
            "initrd": {"name": assets["initrd"]["name"], "hash": assets["initrd"]["hash"]},
            "rootfs": {"name": assets["rootfs"]["name"], "hash": assets["rootfs"]["hash"]},
        },
    }


def _write_registry(tmp_dir: Path, session_dir: Path, contract: dict[str, object]) -> None:
    (tmp_dir / "persistent_registry.json").write_text(
        json.dumps(
            {
                "vms": {
                    SESSION_ID: {
                        "name": SESSION_ID,
                        "profile_id": CODE_PROFILE_ID,
                        "profile_revision": contract["revision"],
                        "profile_payload_hash": "blake3:" + "3" * 64,
                        "asset_pins": contract["pins"],
                        "ram_mb": DEFAULT_RAM_MB,
                        "cpus": DEFAULT_CPUS,
                        "base_version": "0.0.0-ironbank",
                        "created_at": "2026-06-17T00:00:00Z",
                        "session_dir": str(session_dir),
                        "defunct": False,
                    }
                }
            },
            indent=2,
        ),
        encoding="utf-8",
    )


def _stage_fixture_ledger(db_path: Path) -> None:
    """Copy the writer-made fixture ledger and its archive to `db_path`.

    The archive lock is not copied: git stores it 0644, and the archive refuses
    any lock that is not 0600. It is empty, so a fresh one is the same file.
    """
    shutil.copyfile(FIXTURE_DB, db_path)
    source_bodies = FIXTURE_DB.with_suffix(".bodies")
    bodies = db_path.with_suffix(".bodies")
    bodies.mkdir(mode=0o700)
    for generation in source_bodies.iterdir():
        target = bodies / generation.name
        shutil.copyfile(generation, target)
        target.chmod(0o600)
    lock = db_path.with_name(db_path.name + "-archive.lock")
    os.close(os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600))


def _seed_session_db(db_path: Path) -> None:
    """A real ledger with every layer the stats routes project populated.

    The fixture recorded HTTP, model, tool and file activity with archived
    bodies. DNS, exec, audit, credential and rule-match rows are added here,
    through the ledger's own CHECK constraints.
    """
    _stage_fixture_ledger(db_path)
    rule_json = json.dumps(
        {
            "name": "stats_detail_google_detect",
            "action": "allow",
            "detection_level": "informational",
            "match": 'http.host.contains("googleapis.com")',
        },
        sort_keys=True,
    )
    with closing(sqlite3.connect(db_path)) as conn:
        conn.execute(
            """
            INSERT INTO dns_events (
                event_id, timestamp, qname, qtype, qclass, rcode, answer_ip,
                decision, matched_rule, source_proto, process_name,
                upstream_resolver_ms, trace_id, policy_rule, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                DNS_EVENT_ID,
                "2026-06-17T20:11:17Z",
                "daily-cloudcode-pa.googleapis.com",
                1,
                1,
                0,
                "142.250.72.10",
                "allowed",
                "profiles.rules.default_dns",
                "udp",
                "agy",
                29,
                TRACE_ID,
                "profiles.rules.default_dns",
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO fs_events (
                event_id, timestamp, action, path, directory, name, size,
                trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                FILE_EVENT_ID,
                "2026-06-17T20:11:21Z",
                "created",
                "/root/poeme.md",
                "/root",
                "poeme.md",
                96,
                TRACE_ID,
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO exec_events (
                event_id, timestamp, exec_id, command, exit_code, duration_ms,
                stdout_preview, stderr_preview, stdout_bytes, stderr_bytes,
                source, trace_id, process_name, pid, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                EXEC_EVENT_ID,
                "2026-06-17T20:11:16Z",
                7,
                "agy --allow-dangerous-permissions",
                0,
                15,
                "Antigravity CLI 1.0.8",
                "",
                23,
                0,
                "api",
                TRACE_ID,
                "agy",
                215,
                None,
            ),
        )
        conn.execute(
            """
            INSERT INTO audit_events (
                event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                exit_code, session_id, tty, audit_id, exec_event_id, parent_exe,
                trace_id, credential_ref
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "fedcba654321",
                "2026-06-17T20:11:16Z",
                215,
                1,
                0,
                "/usr/local/bin/agy",
                "agy",
                json.dumps(["agy", "--allow-dangerous-permissions"]),
                "/root",
                None,
                1,
                "pts/0",
                "audit-1",
                7,
                "/usr/bin/bash",
                TRACE_ID,
                None,
            ),
        )
        conn.executemany(
            """
            INSERT INTO substitution_events (
                event_id, timestamp, material_class, source, event_type,
                algorithm, substitution_ref, outcome, provider, trace_id,
                context_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    CRED_EVENT_ID,
                    "2026-06-17T20:11:15Z",
                    "credential",
                    "http.body.response.$.access_token",
                    "http.request",
                    "blake3",
                    CREDENTIAL_REF,
                    "captured",
                    "google",
                    TRACE_ID,
                    json.dumps({"domain": "oauth2.googleapis.com"}),
                ),
                (
                    "abc123def457",
                    "2026-06-17T20:11:16Z",
                    "credential",
                    "http.header.authorization",
                    "http.request",
                    "blake3",
                    CREDENTIAL_REF,
                    "injected",
                    "google",
                    TRACE_ID,
                    json.dumps({"domain": "daily-cloudcode-pa.googleapis.com"}),
                ),
            ],
        )
        # On disk a match stores its rule once, in its run, and points at it.
        matches = [
            (
                1_789_000_223_456,
                SEC_EVENT_ID,
                "http.request",
                "profiles.rules.ai_google_http_googleapis",
                "allow",
                "informational",
                rule_json,
            ),
            (
                1_789_000_223_457,
                "223abc456def",
                "mcp.tool_call",
                "profiles.rules.default_mcp",
                "ask",
                "none",
                json.dumps({"name": "default_mcp", "action": "ask"}, sort_keys=True),
            ),
        ]
        for timestamp, event_id, event_type, rule_id, action, level, rule in matches:
            run_id = conn.execute(
                """
                INSERT INTO security_rule_runs (
                    event_type, rule_id, rule_action, detection_level, rule_json,
                    count, first_timestamp_unix_ms, last_timestamp_unix_ms
                ) VALUES (?, ?, ?, ?, ?, 1, ?, ?)
                """,
                (event_type, rule_id, action, level, rule, timestamp, timestamp),
            ).lastrowid
            conn.execute(
                """
                INSERT INTO security_rule_events (
                    timestamp_unix_ms, event_id, event_type, rule_id, rule_action,
                    detection_level, run_id, trace_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (timestamp, event_id, event_type, rule_id, action, level, run_id, TRACE_ID),
            )
        conn.commit()


def _ledger_event_ids(db_path: Path, table: str) -> set[str]:
    with closing(open_session_ledger(db_path)) as conn:
        return {row[0] for row in conn.execute(f"SELECT event_id FROM {table}")}


def _decode(body: dict[str, object]) -> bytes:
    content = str(body["content"])
    if body["encoding"] == "base64":
        return base64.b64decode(content)
    return content.encode()


def test_agy_stats_detail_routes_project_session_db_without_preview_theater() -> None:
    service = ServiceInstance()
    try:
        session_dir = service.tmp_dir / "persistent" / SESSION_ID
        session_dir.mkdir(parents=True, exist_ok=True)
        contract = _profile_contract(service.tmp_dir)
        db_path = session_dir / "session.db"
        _seed_session_db(db_path)
        _write_registry(service.tmp_dir, session_dir, contract)
        # The oracle: what the ledger holds, read by the independent test
        # reader before the service ever opens the file.
        archived = {
            (table, event_id, direction): body
            for table, event_id, direction, body in archived_bodies(db_path)
        }
        assert archived, "the fixture ledger has archived bodies"

        service.start()
        client = service.client()

        detail = client.get(f"/vms/{SESSION_ID}/stats/detail", timeout=30)
        rendered = json.dumps(detail)
        assert "request_body_preview" not in rendered
        assert "response_body_preview" not in rendered

        # Each list is the ledger's rows, not a projection the route invented.
        for field, table in (
            ("http_events", "net_events"),
            ("dns_events", "dns_events"),
            ("file_events", "fs_events"),
            ("process_events", "exec_events"),
            ("credential_events", "substitution_events"),
        ):
            assert {row["event_id"] for row in detail[field]} == _ledger_event_ids(db_path, table), field
        assert {row["event_id"] for row in detail["dns_events"]} == {DNS_EVENT_ID}
        # Newest first, as every detail list is.
        assert [row["verb"] for row in detail["credential_events"]] == ["injected", "captured"]

        # A tool row names the response it got back by that response's own
        # event id, which is where the archive keeps its full text (#245).
        responses = {
            row["response_event_id"] for row in detail["tool_events"] if row["response_event_id"]
        }
        assert responses, "the fixture ledger has tool calls with reported responses"
        assert responses <= _ledger_event_ids(db_path, "tool_responses")

        # Body metadata is the archive index for exactly the events the lists
        # carry: every archived body of a listed event, nothing for an event
        # no list names.
        listed = (
            {
                row["event_id"]
                for field in ("http_events", "model_events", "tool_events", "process_events")
                for row in detail[field]
            }
            | responses
            | _ledger_event_ids(db_path, "security_rule_events")
        )
        route_index = {
            (row["source_table"], event_id, row["direction"]): row
            for event_id, rows in detail["body_blobs"].items()
            for row in rows
        }
        assert set(route_index) == {key for key in archived if key[1] in listed}
        assert {key[0] for key in route_index} >= {"net_events", "model_calls", "tool_calls", "tool_responses"}
        for key, row in route_index.items():
            assert row["original_bytes"] == len(archived[key]), key
            assert row["truncated"] is False, key
            assert "body" not in row and "content" not in row, key

        # A body larger than any preview comes back whole, byte for byte.
        (table, event_id, direction), largest = max(archived.items(), key=lambda item: len(item[1]))
        assert len(largest) > PREVIEW_BYTES
        served = client.get(f"/vms/{SESSION_ID}/bodies/{event_id}", timeout=30)
        assert served["event_id"] == event_id
        body = next(
            row
            for row in served["bodies"]
            if (row["source_table"], row["direction"]) == (table, direction)
        )
        assert body["truncated"] is False
        assert body["truncated_for_transport"] is False
        assert _decode(body) == largest

        latest = client.get(f"/vms/{SESSION_ID}/security/latest?limit=10", timeout=30)
        assert [row["event_id"] for row in latest] == ["223abc456def", SEC_EVENT_ID]
        assert latest[1]["rule_id"] == "profiles.rules.ai_google_http_googleapis"
        assert latest[1]["rule_action"] == "allow"
        assert latest[1]["detection_level"] == "informational"
        # The rule comes back resolved through its run; the matched event's
        # payload is archive-backed and is not part of a ledger row.
        assert json.loads(latest[1]["rule_json"])["name"] == "stats_detail_google_detect"
        assert "event_json" not in latest[1]

        # Status is the writer's counter snapshot, read whole, never a scan of
        # the rows (#223). The two matches above were inserted behind the
        # writer's back, so they are listed but were never counted: the route
        # must report the snapshot, not recount the table to agree with it.
        security = client.get(f"/vms/{SESSION_ID}/security/status", timeout=30)
        with closing(open_session_ledger(db_path)) as conn:
            counted = ledger_counters(conn).get("security", {})
        assert security["total"] == counted.get("matches", 0) < len(latest)
        for field, key in (
            ("by_action", "rule_action"),
            ("by_level", "detection_level"),
            ("by_event_type", "event_type"),
        ):
            assert {row[key]: row["count"] for row in security[field]} == counted.get(field, {}), field

        detection_latest = client.get(f"/vms/{SESSION_ID}/detection/latest?limit=10", timeout=30)
        enforcement_latest = client.get(
            f"/vms/{SESSION_ID}/enforcement/latest?limit=10",
            timeout=30,
        )
        assert detection_latest == [latest[1]]
        assert enforcement_latest == latest
    finally:
        service.stop()
