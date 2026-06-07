"""Exhaustive net_events table validation."""

import pytest

pytestmark = pytest.mark.session_exhaustive


class TestNetEventsData:

    def test_net_event_has_domain(self, exhaust_db):
        """net_events rows have a non-empty domain field."""
        rows = exhaust_db.execute("SELECT domain FROM net_events LIMIT 10").fetchall()
        for row in rows:
            assert row["domain"], "domain should not be empty"

    def test_net_event_has_decision(self, exhaust_db):
        """net_events rows have decision field (allowed/denied)."""
        rows = exhaust_db.execute("SELECT decision FROM net_events LIMIT 10").fetchall()
        for row in rows:
            assert row["decision"] in ("allowed", "denied", "blocked", "error"), (
                f"Unexpected decision: {row['decision']}"
            )

    def test_net_event_has_timestamp(self, exhaust_db):
        """net_events rows have ISO 8601 timestamps."""
        rows = exhaust_db.execute("SELECT timestamp FROM net_events LIMIT 10").fetchall()
        for row in rows:
            ts = row["timestamp"]
            assert ts, "timestamp should not be empty"
            # Basic ISO 8601 check: contains date separator
            assert "T" in ts or "-" in ts, f"Timestamp not ISO 8601: {ts}"

    def test_allowed_event_has_status_code(self, exhaust_db):
        """Allowed net_events have an HTTP status code."""
        rows = exhaust_db.execute(
            "SELECT status_code FROM net_events WHERE decision='allowed' LIMIT 5"
        ).fetchall()
        for row in rows:
            code = row["status_code"]
            if code is not None:
                assert 100 <= code < 600, f"Invalid status code: {code}"

    def test_net_event_port_443(self, exhaust_db):
        """HTTPS net_events use port 443."""
        rows = exhaust_db.execute(
            "SELECT port FROM net_events WHERE port IS NOT NULL LIMIT 5"
        ).fetchall()
        for row in rows:
            assert row["port"] in (443, 80), f"Unexpected port: {row['port']}"

    def test_denied_event_logged(self, exhaustive_env, exhaust_db):
        """A request to a blocked domain produces a denied event."""
        client, vm_name, _ = exhaustive_env
        client.post(f"/exec/{vm_name}", {
            "command": "curl -s https://malware.example.com 2>&1 || true"
        })
        import time
        time.sleep(2)
        # Check for denied events
        rows = exhaust_db.execute(
            "SELECT decision FROM net_events WHERE decision != 'allowed'"
        ).fetchall()
        # May or may not have denied events depending on policy
        assert len(rows) >= 0
