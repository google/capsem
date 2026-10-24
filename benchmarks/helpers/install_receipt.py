"""Project fresh owning reports only after their exact native proof joins."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from urllib.parse import urlsplit

from capsem_builder import crypto
from capsem_builder.gate import config as gate_config
from capsem_builder.release.tools.release_apply_receipt import activation_receipt
from capsem_builder.release.tools.release_glowup import (
    validate_installed_evidence,
    validate_transition_sequence,
)
from capsem_builder.release.tools.release_macos_apply import finish as finish_mac

from .install_inputs import Prepared, phase_proof, read_bytes, read_object

ROOT = Path(__file__).resolve().parents[2]


def pairing(phase: dict, channel: str) -> dict:
    return {
        "channel": channel,
        "manifest_sha256": phase["manifest_sha256"],
        "package_version": phase["artifact"]["version"],
        "package_sha256": phase["artifact"]["sha256"],
        "runtime_sha256": phase["runtime_sha256"],
    }


def verified_receipt(native: Path, prepared: Prepared, *, maximum: int) -> dict:
    report = read_object(native / "report.json", maximum)
    inputs, pair = prepared.inputs, prepared.pair
    if inputs.platform == "macos":
        config = gate_config.load(ROOT)
        sys.path.insert(0, str(config.path(config.install.suite.macos_tart_glowup_script).parent))
        from macos_binary_apply import validate_report

        assert inputs.tart is not None
        checked = validate_report(
            native / "report.json",
            {**pair, "limits": {"maximum": maximum}},
            source=inputs.tart.manifest_url,
        )
        original_transport(checked)
        share = native / "share"
        spec = read_object(share / "binary-apply-pair.json", maximum)
        if any(
            spec.get(key) != pair[key] for key in ("schema", "scope", "channel", "before", "after")
        ):
            raise ValueError("Install original Tart inputs differ from the admitted pair")
        raw = finish_mac(share, spec, source=inputs.tart.manifest_url, maximum=maximum)
        for key in ("original_request", "transition", "installed", "retirement", "capabilities"):
            if raw[key] != checked[key]:
                raise ValueError("Install original Tart proof files differ from its report")
        return checked
    if (
        report.get("schema") != "capsem.release_glowup.v1"
        or report.get("adapter") != "linux-docker-systemd"
    ):
        raise ValueError("Install requires the existing native Linux owner report")
    if report.get("artifact") != pair["after"]["artifact"]:
        raise ValueError("Install native artifact differs from its exact prepared package")
    for name in (
        "native_install",
        "systemd",
        "service_owned_update",
        "full_doctor",
        "installed_winterfell",
    ):
        if report.get("capabilities", {}).get(name) is not True:
            raise ValueError("Install lacks an actual native installed capability")
    validate_installed_evidence(report["installed"])
    if "binary_only" not in report.get("transition_scope", []):
        raise ValueError("Install report did not execute the original binary transition")
    validate_transition_sequence(
        report["transitions"], expected_transitions=report["transition_scope"]
    )
    adapter = report["adapter_evidence"]
    expected = {
        "kind": "binary_only",
        "before": pairing(pair["before"], inputs.channel),
        "after": pairing(pair["after"], inputs.channel),
    }
    if adapter.get("release_pairing") != expected:
        raise ValueError("Install native predecessor/candidate pairing is foreign")
    receipt = read_object(native / "candidate-apply.json", maximum)
    original_transport(receipt)
    if adapter.get("explicit_apply") != receipt or receipt.get("candidate") != expected["after"]:
        raise ValueError("Install native report lacks the exact original Apply receipt")
    source = adapter["polled_manifest_url"]
    url = urlsplit(source)
    if url.scheme != "http" or url.hostname != "127.0.0.1" or url.username or url.password:
        raise ValueError("Install native transport is not the existing loopback owner")
    checked = activation_receipt(
        receipt["original_request"],
        receipt["transition"],
        kind="binary_only",
        source=source,
        digest=receipt["transport_manifest_sha256"],
        source_commit=inputs.source_commit,
    )
    if receipt.get("activation") != checked["activation"]:
        raise ValueError("Install original receipt has no verified activation")
    original_files(
        native,
        receipt,
        marker_name="candidate-after-audit-line",
        request_name="candidate-apply-request.json",
        transition_name="candidate-after-transition.json",
        maximum=maximum,
    )
    for name in ("installed", "doctor", "winterfell"):
        path = native / f"candidate-after-{name}.json"
        raw = read_bytes(path, maximum)
        data = json.loads(raw)
        if crypto.sha256(raw).hexdigest() != receipt["installed_proofs"][name]:
            raise ValueError("Install original native proof bytes changed")
        if name == "installed":
            data.update(package_receipt=True, binary_cohort=True)
            validate_installed_evidence(data)
            if (
                data.get("package_version") != pair["after"]["artifact"]["version"]
                or data.get("manifest_url") != source
            ):
                raise ValueError("Install installed package/transport proof is foreign")
        elif data.get("schema") != f"capsem.installed_{name}.v1" or data.get("passed") is not True:
            raise ValueError("Install doctor/Winterfell proof did not pass")
    return receipt


def original_files(
    root: Path,
    receipt: dict,
    *,
    marker_name: str,
    request_name: str,
    transition_name: str,
    maximum: int,
) -> None:
    marker = int(read_bytes(root / marker_name, maximum).strip())
    if marker < 0 or receipt["original_request"].get("after_line") != marker:
        raise ValueError("Install original request has a foreign audit marker")
    if receipt["original_request"] != read_object(root / request_name, maximum) or receipt[
        "transition"
    ] != read_object(root / transition_name, maximum):
        raise ValueError("Install original request/transition files changed")


def original_transport(receipt: dict) -> str:
    value = receipt["original_request"].get("transport")
    if value not in {"service_uds", "gateway_http"}:
        raise ValueError("Install requires explicit original transport provenance")
    return value


def sidecar(prepared: Prepared, receipt: dict) -> str:
    return json.dumps(
        {
            "schema": "capsem.bench.install.v1",
            "scope": "binary_apply_only",
            "source_commit": prepared.inputs.source_commit,
            "transport": original_transport(receipt),
            "owner": "macos-tart"
            if prepared.inputs.platform == "macos"
            else "linux-docker-systemd",
            "before": phase_proof(prepared.pair["before"]),
            "after": phase_proof(prepared.pair["after"]),
            "receipt_sha256": crypto.sha256(
                json.dumps(receipt, sort_keys=True).encode()
            ).hexdigest(),
        },
        sort_keys=True,
    )
