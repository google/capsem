"""Original Apply transport never inherits the independent activation verdict."""

from __future__ import annotations

import http.server
import json
import socketserver
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.release.tools import release_installed_probe
from capsem_builder.release.tools.release_transition import TransitionEvidenceError
from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport as RealTransport


def test_exact_owner_uses_one_explicit_apply_after_disabling_automatic_polling():
    source = Path(release_installed_probe.__file__).with_name("local_release_glowup.py").read_text()
    assert "release_installed_apply" in source
    assert source.index("disable-automatic") < source.index(
        "record_update_audit_marker(candidate_marker)"
    )
    assert source.count("installed_apply.request_shell(") == 1


def probe():
    from helpers import installed_apply_probe

    return installed_apply_probe


class Transport:
    def __init__(self, response):
        self.response = response
        self.applies = 0

    def measure_request(self, method, path, **kwargs):
        if path == "/settings/info":
            return receipt(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": False}]}
            )
        assert (method, path) == ("POST", "/update/apply")
        assert json.loads(kwargs["body"]) == {"confirmed": True}
        self.applies += 1
        if isinstance(self.response, Exception):
            raise self.response
        return self.response


def receipt(value, status=200):
    return MeasuredResponse(status, {}, json.dumps(value).encode(), 1, 2, 3)


def arguments(tmp_path):
    return SimpleNamespace(
        source="http://127.0.0.1/manifest.json",
        digest="a" * 64,
        source_commit="b" * 40,
        after_line=0,
        audit=tmp_path / "audit",
        output=tmp_path / "request.json",
        timeout=0.05,
        maximum=4096,
        transport="service_uds",
    )


@pytest.mark.parametrize(
    "response,outcome",
    [
        (ConnectionError("secret body must not escape"), "transport_failure"),
        (TimeoutError("private detail"), "transport_failure"),
        (receipt({"error": "installer refused"}, 500), "http_failure"),
        (receipt({"status": "failed", "exit_code": 1}), "application_failure"),
        (receipt({"status": "succeeded", "exit_code": 0}), "succeeded"),
        (receipt({"status": "planned"}), "invalid_response"),
    ],
)
def test_original_request_has_truthful_bounded_outcome(tmp_path, response, outcome):
    owner = Transport(response)
    args = arguments(tmp_path)
    result = probe().request_apply(owner, args)
    assert owner.applies == 1
    assert result["outcome"] == outcome
    assert result["attempts"] == 1
    assert result["candidate_manifest_sha256"] == args.digest
    assert result["source_commit"] == args.source_commit
    assert result["elapsed_ns"] >= 0
    assert "secret" not in args.output.read_text()
    assert "private detail" not in args.output.read_text()
    assert json.loads(args.output.read_text()) == result


def audit_rows(args):
    common = {
        "schema": "capsem.update_audit.v1",
        "source": args.source,
        "candidate_manifest_sha256": args.digest,
    }
    return [
        {**common, "event": "release_candidate_fetched"},
        {
            **common,
            "event": "release_candidate_activated",
            "current": {"manifest_sha256": args.digest},
        },
    ]


def test_preexisting_candidate_winner_never_launches_apply(tmp_path):
    args = arguments(tmp_path)
    args.audit.write_text("\n".join(map(json.dumps, audit_rows(args))))
    owner = Transport(receipt({"status": "succeeded", "exit_code": 0}))
    with pytest.raises(ValueError, match="before original"):
        probe().request_apply(owner, args)
    assert owner.applies == 0


def test_enabled_automatic_updates_refuses_before_post(tmp_path):
    class Enabled(Transport):
        def measure_request(self, *args, **kwargs):
            return receipt(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": True}]}
            )

    owner = Enabled(None)
    with pytest.raises(ValueError, match="automatic"):
        probe().request_apply(owner, arguments(tmp_path))
    assert owner.applies == 0


@pytest.mark.parametrize("response", [ConnectionError("EOF"), receipt({"error": "failed"}, 500)])
def test_failed_original_and_independent_activation_remain_distinct(tmp_path, response):
    args = arguments(tmp_path)
    result = probe().request_apply(Transport(response), args)
    from capsem_builder.release.tools.release_transition import build_transition_verdict

    verdict = build_transition_verdict(
        audit_rows(args),
        kind="binary_only",
        result="activated",
        source=args.source,
        candidate_manifest_sha256=args.digest,
    )
    joined = probe().activation_receipt(
        result,
        verdict,
        kind="binary_only",
        source=args.source,
        digest=args.digest,
        source_commit=args.source_commit,
    )
    assert joined["activation"] == "verified"
    assert joined["original_request"]["outcome"] != "succeeded"
    assert joined["original_request"]["attempts"] == 1


@pytest.mark.parametrize("corruption", ["missing", "digest", "source", "rejected", "commit"])
def test_activation_join_refuses_unproven_or_foreign_outcome(tmp_path, corruption):
    args = arguments(tmp_path)
    result = probe().request_apply(Transport(ConnectionError("EOF")), args)
    from capsem_builder.release.tools.release_transition import build_transition_verdict

    verdict = build_transition_verdict(
        audit_rows(args),
        kind="binary_only",
        result="activated",
        source=args.source,
        candidate_manifest_sha256=args.digest,
    )
    if corruption == "missing":
        verdict = {}
    elif corruption == "commit":
        result["source_commit"] = "c" * 40
    else:
        verdict[
            {"digest": "candidate_manifest_sha256", "source": "source", "rejected": "result"}[
                corruption
            ]
        ] = "wrong"
    with pytest.raises((ValueError, TransitionEvidenceError)):
        probe().activation_receipt(
            result,
            verdict,
            kind="binary_only",
            source=args.source,
            digest=args.digest,
            source_commit=args.source_commit,
        )


@pytest.mark.parametrize(
    "mode,outcome",
    [
        ("eof", "transport_failure"),
        ("truncated", "transport_failure"),
        ("oversize", "transport_failure"),
        ("failed", "http_failure"),
    ],
)
def test_real_uds_original_exchange_is_never_replayed(tmp_path, mode, outcome):
    counts = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, format, *args):
            pass

        def do_GET(self):
            body = receipt(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": False}]}
            ).body
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            counts.append(self.path)
            body = self.rfile.read(int(self.headers["Content-Length"]))
            assert json.loads(body) == {"confirmed": True}
            if mode == "eof":
                self.close_connection = True
                return
            self.send_response(500 if mode == "failed" else 200)
            self.send_header(
                "Content-Length",
                "5000" if mode == "oversize" else "99" if mode == "truncated" else "2",
            )
            self.end_headers()
            if mode != "oversize":
                self.wfile.write(b"{}")
            self.close_connection = True

    # Pytest's worker/node basetemp can exceed sockaddr_un's path limit.
    # Keep evidence there, but own a short-lived socket beside the temp root.
    with (
        tempfile.TemporaryDirectory(prefix="apply-") as directory,
        socketserver.UnixStreamServer(str(Path(directory) / "s"), Handler) as server,
    ):
        worker = threading.Thread(
            target=lambda: server.serve_forever(poll_interval=0.01), daemon=True
        )
        worker.start()
        assert isinstance(server.server_address, str)
        transport = RealTransport(socket_path=server.server_address)
        try:
            args = arguments(tmp_path)
            args.timeout = 1
            result = probe().request_apply(transport, args)
            assert result["outcome"] == outcome
            assert counts == ["/update/apply"]
            assert result["attempts"] == 1
        finally:
            transport.close()
            server.shutdown()
            worker.join(timeout=1)
            assert not worker.is_alive()


def test_automatic_control_verifies_write_and_fresh_read(tmp_path):
    class Settings:
        def __init__(self):
            self.value = True
            self.methods = []

        def measure_request(self, method, path, **kwargs):
            self.methods.append(method)
            if method == "PATCH":
                self.value = json.loads(kwargs["body"])["app.auto_update"]
            return receipt(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": self.value}]}
            )

    owner = Settings()
    probe().set_automatic(owner, arguments(tmp_path), False)
    assert owner.methods == ["PATCH", "GET"]
    assert owner.value is False


def test_automatic_control_rejects_ignored_setting(tmp_path):
    class Ignored:
        def measure_request(self, *args, **kwargs):
            return receipt(
                {"tree": [{"kind": "leaf", "id": "app.auto_update", "effective_value": True}]}
            )

    with pytest.raises(ValueError, match="did not settle"):
        probe().set_automatic(Ignored(), arguments(tmp_path), False)


@pytest.mark.parametrize("interpreter", [sys.executable, "/usr/bin/python3"])
def test_complete_staged_apply_probe_is_stdlib_importable(tmp_path, interpreter):
    from build_system.packaging.macos.macos_candidate_content import stage_guest_scripts

    executable = Path(interpreter)
    if not executable.is_file():
        pytest.skip("host does not provide the guest-compatible system Python")
    version = subprocess.run(
        [interpreter, "-S", "-c", "import sys; print(sys.version.split()[0])"],
        env={},
        capture_output=True,
        text=True,
        timeout=10,
        check=True,
    ).stdout.strip()
    if interpreter == "/usr/bin/python3" and not version.startswith("3.9."):
        pytest.skip(f"system Python is {version}, not the guest-compatible3.9")
    share = tmp_path / "share"
    stage_guest_scripts(Path(__file__).resolve().parents[3], share)
    result = subprocess.run(
        [interpreter, "-S", "-m", "helpers.installed_apply_probe", "--help"],
        cwd=share,
        env={},
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    assert result.returncode == 0, f"Python {version}: {result.stdout}{result.stderr}"
    assert "usage:" in result.stdout


def test_staged_constants_postpone_annotations_for_guest_python39():
    import ast

    path = Path(__file__).resolve().parents[3] / "tests/helpers/constants.py"
    tree = ast.parse(path.read_text(), feature_version=(3, 9))
    assert any(
        isinstance(node, ast.ImportFrom)
        and node.module == "__future__"
        and any(alias.name == "annotations" for alias in node.names)
        for node in tree.body
    ), "guest Python3.9 must not eagerly evaluate modern union annotations"
