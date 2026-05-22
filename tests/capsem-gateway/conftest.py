"""Shared fixtures for capsem-gateway integration tests.

Scope: gateway layer only. These tests cover the TCP-to-UDS proxy shell --
routing, auth, CORS, lifecycle, terminal WebSocket handshake, SPA static
serving -- using a pytest-local `MockServiceHandler` as the UDS backend.
They deliberately do NOT verify that the downstream capsem-service
endpoints behave correctly under real inputs; that correctness is owned
by:

  tests/capsem-service/    (every HTTP handler against the real service)
  tests/capsem-mcp/        (every #[tool] in capsem-mcp against a live
                            capsem-mcp -> capsem-service -> VM chain)
  tests/capsem-e2e/        (full CLI -> gateway -> service -> VM paths
                            for a handful of flagship flows)

If a gateway-proxied response shape changes (e.g. /list returns a new
field), update the mock here AND the corresponding service test in
tests/capsem-service/. If you find yourself writing an assertion about
what the service should return, you're in the wrong directory.
"""

import json
import os
import socket
import socketserver
import tempfile
import threading
import uuid
from http.server import BaseHTTPRequestHandler
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import GatewayInstance, TcpHttpClient

pytestmark = pytest.mark.gateway

# --- Mock capsem-service on UDS ---

MOCK_VMS = {
    "vm-001": {
        "id": "vm-001",
        "pid": 100,
        "name": "dev",
        "status": "Running",
        "persistent": True,
        "ram_mb": DEFAULT_RAM_MB,
        "cpus": DEFAULT_CPUS,
        "version": "0.16.1",
    },
    "vm-002": {
        "id": "vm-002",
        "pid": 200,
        "name": None,
        "status": "Running",
        "persistent": False,
        "ram_mb": DEFAULT_RAM_MB * 2,
        "cpus": DEFAULT_CPUS * 2,
        "version": "0.16.1",
    },
}
MOCK_FILES = {}
MOCK_SKILLS = set()
MOCK_MCP_CONNECTORS = {}
MOCK_RULES = {}


class MockServiceHandler(BaseHTTPRequestHandler):
    """HTTP handler mimicking capsem-service responses."""

    def log_message(self, format, *args):
        pass  # Suppress default logging

    @property
    def clean_path(self):
        """Strip http://localhost prefix that hyper sends over UDS."""
        p = self.path
        if p.startswith("http://localhost"):
            p = p[len("http://localhost"):]
        return p

    def _read_body(self):
        length = int(self.headers.get("Content-Length", 0))
        return self.rfile.read(length) if length > 0 else b""

    def _send_json(self, data, status=200):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_error(self, status, msg):
        self._send_json({"error": msg}, status=status)

    def _send_profile_v2_error(self, status, **data):
        payload = {"mode": "settings_profiles_v2"}
        payload.update(data)
        self._send_json(payload, status=status)

    def do_GET(self):
        parsed = urlparse(self.clean_path)
        if self.clean_path == "/list" or self.clean_path.startswith("/list?"):
            sandboxes = []
            for vm in MOCK_VMS.values():
                sandboxes.append({
                    "id": vm["id"],
                    "pid": vm["pid"],
                    "status": vm["status"],
                    "persistent": vm["persistent"],
                    "ram_mb": vm["ram_mb"],
                    "cpus": vm["cpus"],
                    "profile_id": "everyday-work",
                    "profile_revision": "2026.0520.1",
                    "profile_status": "current",
                })
            self._send_json({
                "sandboxes": sandboxes,
                "asset_health": {
                    "ready": True,
                    "state": "ready",
                    "profile_id": "everyday-work",
                    "profile_revision": "2026.0520.1",
                    "profile_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "profile_assets": [{
                        "logical_name": "rootfs.squashfs",
                        "hash": "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                        "source_url": "https://assets.example.test/rootfs.squashfs",
                        "size": 12,
                        "content_type": "application/vnd.squashfs",
                    }],
                    "missing": [],
                    "retry_count": 0,
                    "retryable": False,
                },
            })
        elif self.clean_path == "/profiles/catalog":
            self._send_json({
                "mode": "settings_profiles_v2",
                "configured_source": "https://profiles.example.test/catalog.json",
                "manifest_present": True,
                "profiles": [{
                    "profile_id": "everyday-work",
                    "current_revision": "2026.0520.1",
                    "installed_revision": "2026.0520.1",
                    "installed_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "revisions": [
                        {
                            "revision": "2026.0520.1",
                            "status": "active",
                            "current": True,
                            "installed": True,
                        },
                        {
                            "revision": "2026.0415.1",
                            "status": "deprecated",
                            "current": False,
                            "installed": False,
                        },
                        {
                            "revision": "2026.0301.1",
                            "status": "revoked",
                            "current": False,
                            "installed": False,
                        },
                    ],
                }],
            })
        elif self.clean_path == "/profiles/everyday-work/revisions":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "current_revision": "2026.0520.1",
                "installed_revision": "2026.0520.1",
                "installed_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "revisions": [
                    {"revision": "2026.0520.1", "status": "active", "current": True, "installed": True},
                    {"revision": "2026.0415.1", "status": "deprecated", "current": False, "installed": False},
                    {"revision": "2026.0301.1", "status": "revoked", "current": False, "installed": False},
                ],
            })
        elif self.clean_path == "/profiles":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profiles": [{
                    "id": "everyday-work",
                    "name": "Everyday Work",
                    "source": "builtin",
                    "locked": True,
                }],
            })
        elif self.clean_path == "/profiles/everyday-work":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile": {"id": "everyday-work", "name": "Everyday Work"},
                "source": "builtin",
                "locked": True,
            })
        elif self.clean_path == "/profiles/everyday-work/effective":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "effective": {
                    "profile_id": "everyday-work",
                    "skills": {"groups": [], "enabled": ["dev-sprint"], "disabled": []},
                },
                "resolver_trace": {"events": []},
            })
        elif self.clean_path == "/confirm/pending":
            self._send_json({
                "mode": "settings_profiles_v2",
                "pending": [],
                "pending_count": 0,
                "resolve_available": False,
                "resolve_owner": "S15-confirm-ux",
            })
        elif parsed.path == "/skills":
            enabled = sorted(MOCK_SKILLS)
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "groups": [],
                "enabled": enabled,
                "disabled": [],
                "skills": [
                    {
                        "id": skill,
                        "kind": "enabled",
                        "source_profile": "everyday-work",
                        "source": "user",
                        "direct": True,
                        "editable": True,
                    }
                    for skill in enabled
                ],
            })
        elif parsed.path == "/mcp/connectors":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "servers": [
                    {
                        "id": server_id,
                        "server": server,
                        "source_profile": "everyday-work",
                        "source": "user",
                        "direct": True,
                        "editable": True,
                    }
                    for server_id, server in sorted(MOCK_MCP_CONNECTORS.items())
                ],
            })
        elif parsed.path == "/rules":
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "rules": list(MOCK_RULES.values()),
            })
        elif self.clean_path == "/setup/assets":
            self._send_json({
                "ready": False,
                "state": "updating",
                "downloading": True,
                "asset_locations": {
                    "assets_dir": "/tmp/capsem-assets",
                    "image_roots": ["/tmp/capsem-images"],
                },
                "asset_version": "everyday-work@2026.0520.1",
                "profile_id": "everyday-work",
                "profile_revision": "2026.0520.1",
                "profile_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "profile_assets": [{
                    "logical_name": "rootfs.squashfs",
                    "hash": "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "source_url": "https://assets.example.test/rootfs.squashfs",
                    "size": 12,
                    "content_type": "application/vnd.squashfs",
                }],
                "arch": "arm64",
                "missing": ["rootfs.squashfs"],
                "progress": {
                    "logical_name": "rootfs.squashfs",
                    "bytes_done": 6,
                    "bytes_total": 12,
                    "done": False,
                },
                "error": None,
                "retry_count": 0,
                "retryable": False,
                "assets": [
                    {
                        "name": "vmlinuz",
                        "path": "/tmp/capsem-assets/vmlinuz",
                        "status": "present",
                    },
                    {
                        "name": "initrd.img",
                        "path": "/tmp/capsem-assets/initrd.img",
                        "status": "present",
                    },
                    {
                        "name": "rootfs.squashfs",
                        "path": "/tmp/capsem-assets/rootfs.squashfs",
                        "status": "downloading",
                    },
                ],
            })
        elif self.clean_path == "/debug/report":
            self._send_json({
                "text": "\n".join([
                    "Capsem Debug Report",
                    "schema: capsem.debug.v2",
                    "profile_asset_profile_id: everyday-work",
                    "profile_asset_profile_revision: 2026.0520.1",
                    "profile_asset_source: rootfs.squashfs hash=blake3:cccc",
                    "vm_profile_pin: vm-001 everyday-work@2026.0520.1 current",
                    "gateway_runtime_issue: none",
                ]),
                "json": {
                    "schema": "capsem.debug.v2",
                    "redacted": True,
                    "assets": {
                        "source": "profile_v2_asset_health",
                        "health": {
                            "ready": False,
                            "state": "updating",
                            "profile_id": "everyday-work",
                            "profile_revision": "2026.0520.1",
                            "profile_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                            "profile_assets": [{
                                "logical_name": "rootfs.squashfs",
                                "hash": "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                                "source_url": "https://assets.example.test/rootfs.squashfs",
                                "size": 12,
                                "content_type": "application/vnd.squashfs",
                            }],
                        },
                    },
                    "status": {
                        "issues": [],
                        "defunct_sessions": [],
                    },
                },
            })
        elif self.clean_path.startswith("/info/"):
            vm_id = self.clean_path.split("/info/", 1)[1].split("?")[0]
            if vm_id in MOCK_VMS:
                self._send_json(MOCK_VMS[vm_id])
            else:
                self._send_error(404, f"sandbox {vm_id} not found")
        elif self.clean_path.startswith("/logs/"):
            self._send_json({
                "logs": "mock boot log\n",
                "serial_logs": None,
                "process_logs": None,
                "security_logs": (
                    '{"target":"security.event","fields":'
                    '{"message":"resolved_security_event","event_id":"evt-gw-log"}}\n'
                ),
            })
        elif parsed.path.startswith("/files/") and parsed.path.endswith("/content"):
            parts = parsed.path.strip("/").split("/")
            if len(parts) >= 3:
                vm_id = parts[1]
                query = parse_qs(parsed.query)
                rel_path = query.get("path", [""])[0]
                key = (vm_id, rel_path)
                if key not in MOCK_FILES:
                    self._send_error(404, "file not found")
                    return
                data = MOCK_FILES[key]
                self.send_response(200)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            else:
                self._send_error(404, f"unknown endpoint: {self.clean_path}")
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_POST(self):
        body = self._read_body()
        parsed = urlparse(self.clean_path)
        if self.clean_path == "/provision":
            data = json.loads(body) if body else {}
            vm_id = f"vm-{uuid.uuid4().hex[:8]}"
            self._send_json({
                "id": vm_id,
                "profile_id": data.get("profile_id", "everyday-work"),
                "profile_revision": data.get("profile_revision", "2026.0520.1"),
                "profile_status": "current",
                "profile_pin": {
                    "profile_id": data.get("profile_id", "everyday-work"),
                    "profile_revision": data.get("profile_revision", "2026.0520.1"),
                    "profile_payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "package_contract_hash": "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "base_assets": {
                        "version": "everyday-work",
                        "arch": "arm64",
                        "kernel_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "initrd_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "rootfs_hash": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    },
                },
                "asset_health": {
                    "ready": True,
                    "state": "ready",
                    "profile_id": data.get("profile_id", "everyday-work"),
                    "profile_revision": data.get("profile_revision", "2026.0520.1"),
                    "missing": [],
                },
            })
        elif self.clean_path == "/profiles":
            data = json.loads(body) if body else {}
            if not data.get("id"):
                self._send_profile_v2_error(
                    400,
                    error="profile validation failed: id is required",
                    code="profile_invalid",
                    field="id",
                )
                return
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile": data,
                "source": "user",
                "locked": False,
            })
        elif self.clean_path == "/profiles/everyday-work/fork":
            data = json.loads(body) if body else {}
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile": {"id": data.get("id", "forked"), "name": data.get("name", "Forked")},
                "source": "user",
                "locked": False,
            })
        elif self.clean_path == "/profiles/catalog/reconcile":
            self._send_json({
                "mode": "settings_profiles_v2",
                "summary": {
                    "installed": 1,
                    "unchanged": 0,
                    "deprecated_kept": 0,
                    "revoked_removed": 0,
                    "absent_removed": 0,
                    "errors": 0,
                },
                "outcomes": [{
                    "profile_id": "everyday-work",
                    "revision": "2026.0520.1",
                    "outcome": "installed",
                    "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                }],
            })
        elif self.clean_path == "/profiles/revoked-work/revisions/install":
            self._send_json({
                "error": "profile revision is revoked",
                "mode": "settings_profiles_v2",
                "profile_id": "revoked-work",
                "revision": "2026.0301.1",
                "status": "revoked",
            }, status=409)
        elif self.clean_path == "/profiles/everyday-work/revisions/install":
            self._send_json({
                "mode": "settings_profiles_v2",
                "action": "install",
                "profile_id": "everyday-work",
                "selected_revision": "2026.0520.1",
                "requested_revision": None,
                "summary": {"installed": 1, "errors": 0},
                "outcome": {"profile_id": "everyday-work", "revision": "2026.0520.1", "outcome": "installed"},
            })
        elif self.clean_path == "/profiles/everyday-work/revisions/update":
            self._send_json({
                "mode": "settings_profiles_v2",
                "action": "update",
                "profile_id": "everyday-work",
                "selected_revision": "2026.0520.1",
                "summary": {"unchanged": 1, "errors": 0},
                "outcome": {"profile_id": "everyday-work", "revision": "2026.0520.1", "outcome": "unchanged"},
            })
        elif self.clean_path == "/profiles/everyday-work/revisions/remove":
            self._send_json({
                "mode": "settings_profiles_v2",
                "action": "remove",
                "profile_id": "everyday-work",
                "selected_revision": "2026.0520.1",
                "outcome": {"outcome": "removed"},
            })
        elif parsed.path == "/skills":
            data = json.loads(body) if body else {}
            skill_id = data.get("id", "unnamed-skill")
            MOCK_SKILLS.add(skill_id)
            self._send_json({
                "id": skill_id,
                "kind": data.get("kind", "enabled"),
                "source_profile": "everyday-work",
                "source": "user",
                "direct": True,
                "editable": True,
            })
        elif parsed.path == "/mcp/connectors":
            data = json.loads(body) if body else {}
            server_id = data.get("id", "mock")
            server = data.get("server") or data.get("connector") or {"command": "npx", "args": []}
            MOCK_MCP_CONNECTORS[server_id] = server
            self._send_json({
                "id": server_id,
                "server": server,
                "source_profile": "everyday-work",
                "source": "user",
                "direct": True,
                "editable": True,
            })
        elif parsed.path == "/rules":
            data = json.loads(body) if body else {}
            rule_id = data.get("id", "security.rules.http.ask_probe")
            rule = {
                "id": rule_id,
                "callback": data.get("callback", "http.request"),
                "decision": data.get("decision", "ask"),
                "source_profile": "everyday-work",
                "editable": True,
            }
            MOCK_RULES[rule_id] = rule
            self._send_json(rule)
        elif parsed.path == "/rules/evaluate":
            data = json.loads(body) if body else {}
            if data.get("callback") == "bad.callback":
                self._send_profile_v2_error(
                    400,
                    error="unsupported policy callback 'bad.callback'",
                    code="rule_evaluate_invalid_callback",
                    callback="bad.callback",
                )
                return
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "matched_rule_id": "security.rules.http.ask_probe",
                "decision": "ask",
                "would_ask": True,
                "reason": "mock ask",
                "enforced": False,
            })
        elif parsed.path == "/enforcement/validate":
            data = json.loads(body) if body else {}
            self._send_json({
                "compiled": True,
                "id": data.get("id", "block-gateway"),
                "compiled_plan": "cel",
            })
        elif parsed.path.startswith("/sessions/") and parsed.path.endswith("/detection/hunt"):
            data = json.loads(body) if body else {}
            rule = (data.get("rules") or [{}])[0]
            self._send_json({
                "total_matches": 1,
                "unique_evidence_matches": 1,
                "truncated": False,
                "rows": [{
                    "event_ref": {
                        "corpus": "session_db",
                        "session_id": "vm-001",
                        "event_id": "evt-gateway-hunt",
                        "sequence_no": None,
                        "timestamp_unix_ms": 1700000000000,
                    },
                    "rule_id": rule.get("id", "detect-gateway"),
                    "pack_id": rule.get("pack_id", "runtime-detection"),
                    "evidence_signature": "gateway-hunt-evidence",
                    "matched_fields": [{
                        "path": "http.request.host",
                        "value": "example.test",
                    }],
                    "outcome": "matched",
                }],
            })
        elif self.clean_path.startswith("/exec/"):
            data = json.loads(body) if body else {}
            cmd = data.get("command", "")
            self._send_json({"stdout": f"mock: {cmd}\n", "stderr": "", "exit_code": 0})
        elif self.clean_path.startswith("/stop/"):
            self._send_json({"ok": True})
        elif parsed.path.startswith("/files/") and parsed.path.endswith("/content"):
            parts = parsed.path.strip("/").split("/")
            if len(parts) >= 3:
                vm_id = parts[1]
                query = parse_qs(parsed.query)
                rel_path = query.get("path", [""])[0]
                MOCK_FILES[(vm_id, rel_path)] = body
                self._send_json({"success": True, "size": len(body)})
            else:
                self._send_error(404, f"unknown endpoint: {self.clean_path}")
        elif self.clean_path.startswith("/inspect/"):
            self._send_json({"columns": [], "rows": []})
        elif self.clean_path.startswith("/persist/"):
            self._send_json({"ok": True})
        elif self.clean_path == "/purge":
            self._send_json({"purged": 0, "persistent_purged": 0, "ephemeral_purged": 0})
        elif self.clean_path == "/run":
            self._send_json({"stdout": "mock run output\n", "stderr": "", "exit_code": 0})
        elif self.clean_path.startswith("/resume/"):
            self._send_json({"id": "vm-resumed"})
        elif self.clean_path.startswith("/fork/"):
            data = json.loads(body) if body else {}
            self._send_json({"name": data.get("name", "fork"), "size_bytes": 1024})
        elif self.clean_path == "/reload-config":
            self._send_json({"ok": True})
        elif self.clean_path == "/setup/assets/cleanup":
            self._send_profile_v2_error(
                409,
                error="asset cleanup is blocked while assets are updating; retry once assets are ready",
                code="asset_cleanup_blocked",
                asset_state="updating",
            )
        elif self.clean_path == "/echo":
            # Echo back the request body for proxy testing
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_PUT(self):
        body = self._read_body()
        if self.clean_path == "/profiles/everyday-work":
            data = json.loads(body) if body else {}
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile": data,
                "source": "user",
                "locked": False,
            })
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")

    def do_DELETE(self):
        if self.clean_path.startswith("/delete/"):
            self._send_json({"ok": True})
        elif self.clean_path.startswith("/images/"):
            self._send_json({"ok": True})
        elif self.clean_path.startswith("/skills/dev-sprint"):
            self._send_profile_v2_error(
                409,
                error="skill_is_locked: skill 'dev-sprint' is inherited from profile 'everyday-work'",
                code="skill_is_locked",
                profile_id="everyday-work",
                skill_id="dev-sprint",
                kind="enabled",
            )
        elif self.clean_path.startswith("/skills/"):
            skill_id = self.clean_path.split("/skills/", 1)[1].split("?")[0]
            MOCK_SKILLS.discard(skill_id)
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "skill_id": skill_id,
                "kind": "enabled",
                "removed": True,
            })
        elif self.clean_path.startswith("/mcp/connectors/builtin-local"):
            self._send_profile_v2_error(
                409,
                error="server_is_locked: MCP server 'builtin-local' is inherited from profile 'everyday-work'",
                code="server_is_locked",
                profile_id="everyday-work",
                connector_id="builtin-local",
            )
        elif self.clean_path.startswith("/mcp/connectors/"):
            connector_id = self.clean_path.split("/mcp/connectors/", 1)[1].split("?")[0]
            MOCK_MCP_CONNECTORS.pop(connector_id, None)
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "connector_id": connector_id,
                "removed": True,
            })
        elif self.clean_path.startswith("/rules/security.rules.http.default_read"):
            self._send_profile_v2_error(
                409,
                error="rule_is_builtin: rule 'security.rules.http.default_read' is inherited from profile 'everyday-work'",
                code="rule_is_builtin",
                profile_id="everyday-work",
                rule_id="security.rules.http.default_read",
            )
        elif self.clean_path.startswith("/rules/"):
            rule_id = self.clean_path.split("/rules/", 1)[1].split("?")[0]
            MOCK_RULES.pop(rule_id, None)
            self._send_json({
                "mode": "settings_profiles_v2",
                "profile_id": "everyday-work",
                "rule_id": rule_id,
                "removed": True,
            })
        else:
            self._send_error(404, f"unknown endpoint: {self.clean_path}")


class UnixStreamServer(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    allow_reuse_address = True
    daemon_threads = True
    request_queue_size = 128


class MockServiceServer:
    """HTTP server on Unix socket mimicking capsem-service."""

    def __init__(self):
        self.tmp_dir = tempfile.mkdtemp(prefix="capsem-mock-svc-")
        self.socket_path = os.path.join(self.tmp_dir, "service.sock")
        self._server = None
        self._thread = None

    def start(self):
        self._server = UnixStreamServer(self.socket_path, MockServiceHandler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self):
        if self._server:
            self._server.shutdown()
            self._server.server_close()
        if os.path.exists(self.socket_path):
            os.unlink(self.socket_path)


# --- Fixtures ---


@pytest.fixture(scope="session")
def mock_service():
    """Start a mock capsem-service on a Unix socket."""
    svc = MockServiceServer()
    svc.start()
    yield svc
    svc.stop()


@pytest.fixture(scope="session")
def gateway_env(mock_service):
    """Start capsem-gateway binary pointing at the mock UDS."""
    gw = GatewayInstance(uds_path=mock_service.socket_path)
    gw.start()
    yield gw
    gw.stop()


@pytest.fixture
def gw_client(gateway_env):
    """TcpHttpClient with valid auth token."""
    return TcpHttpClient(gateway_env.base_url, gateway_env.token)


@pytest.fixture(scope="session")
def frontend_dir():
    """Create a temp dir with mock frontend build artifacts."""
    d = Path(tempfile.mkdtemp(prefix="capsem-frontend-test-"))
    (d / "index.html").write_text(
        '<!DOCTYPE html><html><head>'
        '<link rel="stylesheet" href="/app/_astro/style.abc.css">'
        '</head><body>'
        '<script type="module" src="/app/_astro/app.xyz.js"></script>'
        '</body></html>'
    )
    astro = d / "_astro"
    astro.mkdir()
    (astro / "style.abc.css").write_text("body { color: red; }")
    (astro / "app.xyz.js").write_text("console.log('capsem');")
    (d / "favicon.ico").write_bytes(b"\x00\x00\x01\x00")
    fonts = d / "fonts"
    fonts.mkdir()
    (fonts / "inter.woff2").write_bytes(b"\x00woff2")
    vm = d / "vm" / "terminal"
    vm.mkdir(parents=True)
    (vm / "index.html").write_text("<html><body>terminal</body></html>")
    yield d
    import shutil
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="session")
def frontend_gateway_env(mock_service, frontend_dir):
    """Gateway started with --frontend-dir pointing at mock assets."""
    gw = GatewayInstance(
        uds_path=mock_service.socket_path,
        frontend_dir=frontend_dir,
    )
    gw.start()
    yield gw
    gw.stop()


@pytest.fixture
def fe_client(frontend_gateway_env):
    """TcpHttpClient for the frontend-enabled gateway."""
    return TcpHttpClient(frontend_gateway_env.base_url,
                         frontend_gateway_env.token)
