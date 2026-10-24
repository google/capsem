"""Existing Linux installed fixture's explicit request and separate receipts."""

from __future__ import annotations

import json
import shlex
import sys
import time
from pathlib import Path

from capsem_builder.gate import config as gate_config

from ... import crypto
from .release_apply_receipt import activation_receipt
from .release_glowup import PairingIdentity


def shell(root: Path, command: str, output: Path, *arguments: str) -> str:
    config = gate_config.load(root)
    # Reuse the standard strict test transport, already in the sealed source
    # image. No installed binary replacement, fresh executable or TCP authority.
    pythonpath = str(Path(config.install.suite.path).parent)
    argv = [
        "env",
        f"PYTHONPATH={pythonpath}",
        sys.executable,
        "-m",
        "helpers.installed_apply_probe",
        command,
        "--socket",
        str(Path.home() / ".capsem" / "run" / "service.sock"),
        "--output",
        str(output),
        *arguments,
    ]
    return shlex.join(argv)


def request_shell(
    root: Path, evidence: Path, *, source: str, digest: str, source_commit: str
) -> str:
    return shell(
        root,
        "request",
        evidence / "candidate-apply-request.json",
        "--source",
        source,
        "--digest",
        digest,
        "--source-commit",
        source_commit,
        "--audit",
        str(Path.home() / ".capsem" / "logs" / "update.log"),
        "--after-line-file",
        str(evidence / "candidate-after-audit-line"),
    )


def complete(
    evidence: Path,
    *,
    expected: PairingIdentity,
    kind: str,
    source: str,
    digest: str,
    source_commit: str,
) -> None:
    from .local_release_glowup import _probe_report_passed, _validate_exact_installed_state

    _validate_exact_installed_state(evidence / "candidate-after-installed.json", expected)
    for name, schema in (
        ("doctor", "capsem.installed_doctor.v1"),
        ("winterfell", "capsem.installed_winterfell.v1"),
    ):
        _probe_report_passed(evidence / f"candidate-after-{name}.json", schema)
    request = json.loads((evidence / "candidate-apply-request.json").read_text())
    marker = int((evidence / "candidate-after-audit-line").read_text().strip())
    if marker < 0 or request.get("after_line") != marker:
        raise ValueError("original Apply request has a different audit marker")
    verdict = json.loads((evidence / "candidate-after-transition.json").read_text())
    result = activation_receipt(
        request, verdict, kind=kind, source=source, digest=digest, source_commit=source_commit
    )
    if expected.manifest_sha256 != digest:
        raise ValueError("Apply pairing does not match the exact candidate manifest")
    result["candidate"] = expected.as_report()
    # Caller has already validated these through the original installed state,
    # doctor and Winterfell owners. Hash the exact retained proof bytes.
    result["installed_proofs"] = {
        name: crypto.sha256((evidence / f"candidate-after-{name}.json").read_bytes()).hexdigest()
        for name in ("installed", "doctor", "winterfell")
    }
    elapsed = time.monotonic_ns() - request["started_ns"]
    if elapsed < request["elapsed_ns"]:
        raise ValueError("activation receipt monotonic clock predates original request")
    result["activation_verified_elapsed_ns"] = elapsed
    (evidence / "candidate-apply.json").write_text(json.dumps(result, sort_keys=True) + "\n")
