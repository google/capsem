"""Finite evidence reads and independent installed native verdict binding."""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest
from capsem_builder.release.tools import release_installed_apply as apply
from capsem_builder.release.tools.release_glowup import PairingIdentity
from capsem_builder.release.tools.release_update_audit import (
    TransitionEvidenceError,
    load_update_audit,
)


def test_audit_read_is_bounded_inside_the_original_reader(tmp_path):
    path = tmp_path / "audit"
    path.write_bytes(b"{}\n" * 20)
    with pytest.raises(TransitionEvidenceError, match="byte budget"):
        load_update_audit(path, max_bytes=0)
    with pytest.raises(TransitionEvidenceError, match="exceeds"):
        load_update_audit(path, max_bytes=20)
    assert load_update_audit(path, after_line=19, max_bytes=60) == [{}]
    assert load_update_audit(tmp_path / "missing", max_bytes=20) == []


def proof_files(root: Path):
    digest = "a" * 64
    expected = PairingIdentity(
        channel="nightly",
        manifest_sha256=digest,
        package_version="1.5.100",
        package_sha256="c" * 64,
        runtime_sha256="d" * 64,
    )
    arguments = {
        "expected": expected,
        "kind": "binary_only",
        "source": "http://127.0.0.1/manifest.json",
        "digest": digest,
        "source_commit": "b" * 40,
    }
    request = {
        "schema": "capsem.installed_apply_request.v1",
        "source": arguments["source"],
        "candidate_manifest_sha256": digest,
        "source_commit": arguments["source_commit"],
        "attempts": 1,
        "after_line": 0,
        "started_ns": time.monotonic_ns(),
        "elapsed_ns": 0,
        "outcome": "transport_failure",
        "error_type": "ConnectionError",
    }
    verdict = {
        "schema": "capsem.release_transition_verdict.v1",
        "kind": "binary_only",
        "result": "activated",
        "source": arguments["source"],
        "candidate_manifest_sha256": digest,
        "installed_manifest_sha256": digest,
        "fetched": True,
        "preserved_previous": False,
    }
    installed = {
        "package_version": "1.5.100",
        "channel": "nightly",
        "manifest_url": arguments["source"],
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
    }
    documents = {
        "candidate-apply-request": request,
        "candidate-after-transition": verdict,
        "candidate-after-installed": installed,
        "candidate-after-doctor": {"schema": "capsem.installed_doctor.v1", "passed": True},
        "candidate-after-winterfell": {"schema": "capsem.installed_winterfell.v1", "passed": True},
    }
    for name, value in documents.items():
        (root / f"{name}.json").write_text(json.dumps(value))
    (root / "candidate-after-audit-line").write_text("0\n")
    return arguments


def test_finish_keeps_original_failure_and_exact_candidate_native_receipts(tmp_path):
    arguments = proof_files(tmp_path)
    apply.complete(tmp_path, **arguments)
    result = json.loads((tmp_path / "candidate-apply.json").read_text())
    assert result["activation"] == "verified"
    assert result["original_request"]["outcome"] == "transport_failure"
    assert result["candidate"] == arguments["expected"].as_report()
    assert set(result["installed_proofs"]) == {"installed", "doctor", "winterfell"}
    assert all(len(digest) == 64 for digest in result["installed_proofs"].values())
    assert result["activation_verified_elapsed_ns"] >= 0


@pytest.mark.parametrize("proof", ["installed", "doctor", "winterfell", "transition"])
def test_finish_cannot_promote_missing_native_proof(tmp_path, proof):
    arguments = proof_files(tmp_path)
    (tmp_path / f"candidate-after-{proof}.json").unlink()
    with pytest.raises((SystemExit, FileNotFoundError)):
        apply.complete(tmp_path, **arguments)
    assert not (tmp_path / "candidate-apply.json").exists()


def test_failed_native_probe_never_becomes_verified_activation(tmp_path):
    arguments = proof_files(tmp_path)
    (tmp_path / "candidate-after-winterfell.json").write_text(
        json.dumps({"schema": "capsem.installed_winterfell.v1", "passed": False})
    )
    with pytest.raises(SystemExit, match="probe failed"):
        apply.complete(tmp_path, **arguments)
    assert not (tmp_path / "candidate-apply.json").exists()


def test_request_and_native_verdict_must_share_original_audit_marker(tmp_path):
    arguments = proof_files(tmp_path)
    (tmp_path / "candidate-after-audit-line").write_text("1\n")
    with pytest.raises(ValueError, match="marker"):
        apply.complete(tmp_path, **arguments)
    assert not (tmp_path / "candidate-apply.json").exists()


def test_activation_cannot_relabel_failed_http_as_original_success(tmp_path):
    arguments = proof_files(tmp_path)
    request = tmp_path / "candidate-apply-request.json"
    data = json.loads(request.read_text())
    data.update(outcome="succeeded", http_status=500)
    request.write_text(json.dumps(data))
    with pytest.raises(ValueError, match="HTTP"):
        apply.complete(tmp_path, **arguments)
    assert not (tmp_path / "candidate-apply.json").exists()
