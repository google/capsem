"""Real portable package/runtime bytes for source-only admission contracts."""

from __future__ import annotations

import io
import json
import tarfile
from pathlib import Path

import blake3
from capsem_builder import crypto
from capsem_builder.release.tools import package_payload, release_inputs

from build_system.tests.packaging.test_macos_binary_apply import cohort


def prepared(root: Path, monkeypatch, *, platform: str = "macos") -> tuple[Path, dict]:
    monkeypatch.setattr(package_payload.shutil, "which", lambda _name: None)
    architecture = "arm64" if platform == "macos" else "amd64"
    payload = b"source-only runtime fixture; not boot evidence"
    image = {
        "name": "rootfs",
        "kind": "rootfs",
        "url": "https://release.invalid/rootfs",
        "bytes": len(payload),
        "digest": {
            "sha256": crypto.sha256(payload).hexdigest(),
            "blake3": blake3.blake3(payload).hexdigest(),
        },
    }
    runtime = {
        "revision": "same-fixture-runtime",
        "min_capsem_version": "0.7.0",
        "architectures": [{"architecture": architecture, "images": [image]}],
    }
    result = {
        "schema": "capsem.bench.install_inputs.v1",
        "platform": platform,
        "architecture": architecture,
        "source_commit": "b" * 40,
        "channel": "nightly",
        "content_root": str(root),
    }
    for phase in ("before", "after"):
        package, manifest, sbom = cohort(root, phase)
        document = json.loads(manifest.read_text())
        document["runtime"] = runtime
        if platform == "linux":
            files = package_payload.package_payload_files(package)
            destination = package.with_name(
                f"Capsem_{document['packages'][0]['version']}_amd64.deb"
            )
            archive = io.BytesIO()
            with tarfile.open(fileobj=archive, mode="w:gz") as tar:
                for name, data in files.items():
                    info = tarfile.TarInfo(name.lstrip("/"))
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
            data = archive.getvalue()
            header = (
                f"{'data.tar.gz/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n".encode()
            )
            destination.write_bytes(
                b"!<arch>\n" + header + data + (b"\n" if len(data) % 2 else b"")
            )
            package = destination
            row = document["packages"][0]
            row.update(
                name=package.name,
                platform=platform,
                architecture=architecture,
                bytes=package.stat().st_size,
                digest={"sha256": crypto.sha256(package.read_bytes()).hexdigest()},
            )
            for binary in row["binaries"]:
                binary.update(platform=platform, architecture=architecture)
        manifest.write_text(json.dumps(document))
        directory = manifest.parent / "runtime-inputs"
        directory.mkdir()
        (directory / "manifest.json").write_bytes(manifest.read_bytes())
        rows = release_inputs.resolved_artifact_rows(
            document, "https://release.invalid/manifest.json", "runtime", architecture=architecture
        )
        for row in rows:
            target = directory / row["relative"]
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(payload)
        report = {
            "schema": "capsem.release_inputs.v1",
            "kind": "runtime",
            "architecture": architecture,
            "manifest_url": "https://release.invalid/manifest.json",
            "artifacts": release_inputs.report_artifacts(rows),
        }
        (directory / "release-inputs.json").write_text(json.dumps(report))
        result[phase] = {
            "package": str(package),
            "manifest": str(manifest),
            "sbom": str(sbom),
            "runtime_inputs": str(directory),
        }
    if platform == "macos":
        result["tart"] = {
            "asset_share": str(root),
            "manifest_url": "http://127.0.0.1:18765/candidate/assets/nightly/manifest.json",
        }
        for name in ("tampered_manifest", "updated_manifest", "incompatible_manifest"):
            path = root / f"{name}.json"
            path.write_text(json.dumps({"fixture": name}))
            result["tart"][name] = str(path)
    path = root / "inputs.json"
    path.write_text(json.dumps(result))
    return path, result


def load(path: Path, declaration: dict):
    from benchmarks.helpers.install_inputs import load_inputs

    return load_inputs(
        path,
        maximum=1048576,
        package_maximum=1048576,
        source_commit="b" * 40,
        platform=declaration["platform"],
        architecture=declaration["architecture"],
    )


def linux_receipt(root: Path, selected):
    """Canonical source proof builders over controlled bytes; no native claim."""
    from capsem_builder.release.tools import release_glowup as proof
    from capsem_builder.release.tools import release_installed_apply
    from capsem_builder.release.tools.release_apply_receipt import system_monotonic_ns

    from benchmarks.helpers.install_receipt import pairing

    root.mkdir()
    inputs, pair = selected.inputs, selected.pair
    before = proof.PairingIdentity(**pairing(pair["before"], inputs.channel))
    after = proof.PairingIdentity(**pairing(pair["after"], inputs.channel))
    source, digest = "http://127.0.0.1:18765/manifest.json", "f" * 64
    installed = {
        "package_version": after.package_version,
        "channel": inputs.channel,
        "manifest_url": source,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
        "package_receipt": True,
        "binary_cohort": True,
    }
    request = {
        "schema": "capsem.installed_apply_request.v1",
        "source": source,
        "source_commit": inputs.source_commit,
        "candidate_manifest_sha256": digest,
        "after_line": 4,
        "attempts": 1,
        "outcome": "transport_failure",
        "transport": "service_uds",
        "error_type": "ConnectionError",
        "started_ns": system_monotonic_ns() - 2000000,
        "elapsed_ns": 1000000,
    }
    transition = {
        "schema": "capsem.release_transition_verdict.v1",
        "kind": "binary_only",
        "result": "activated",
        "source": source,
        "candidate_manifest_sha256": digest,
        "installed_manifest_sha256": digest,
        "fetched": True,
        "preserved_previous": False,
    }
    for name, data in {
        "candidate-apply-request": request,
        "candidate-after-transition": transition,
        "candidate-after-installed": installed,
        "candidate-after-doctor": {"schema": "capsem.installed_doctor.v1", "passed": True},
        "candidate-after-winterfell": {"schema": "capsem.installed_winterfell.v1", "passed": True},
    }.items():
        (root / (name + ".json")).write_text(json.dumps(data))
    (root / "candidate-after-audit-line").write_text("4\n")
    release_installed_apply.complete(
        root,
        expected=after,
        kind="binary_only",
        source=source,
        digest=digest,
        authoritative_digest=after.manifest_sha256,
        source_commit=inputs.source_commit,
    )
    receipt = json.loads((root / "candidate-apply.json").read_text())
    transitions = [
        proof.build_transition_evidence(
            kind=proof.TransitionKind.FRESH_INSTALL,
            before=None,
            after=before,
            result="activated",
            doctor_passed=True,
            winterfell_passed=True,
        ),
        proof.build_transition_evidence(
            kind=proof.TransitionKind.BINARY_ONLY,
            before=before,
            after=after,
            result="activated",
            doctor_passed=True,
            winterfell_passed=True,
        ),
    ]
    artifact = proof.artifact_identity_from_manifest_package(
        inputs.after.manifest.read_bytes(), inputs.after.package
    )
    report = proof.build_report(
        adapter="linux-docker-systemd",
        artifact=artifact,
        installed=installed,
        capabilities=dict.fromkeys(
            (
                "native_install",
                "systemd",
                "service_owned_update",
                "full_doctor",
                "installed_winterfell",
            ),
            True,
        ),
        transitions=transitions,
        expected_transitions=("fresh_install", "binary_only"),
    )
    report["adapter_evidence"] = {
        "release_pairing": {
            "kind": "binary_only",
            "before": before.as_report(),
            "after": after.as_report(),
        },
        "polled_manifest_url": source,
        "explicit_apply": receipt,
    }
    (root / "report.json").write_text(json.dumps(report))
    return report
