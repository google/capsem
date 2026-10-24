"""Refusal adapter source facts are not native install/compatibility proof."""

from __future__ import annotations

import copy
import os
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.release.tools import release_admission_refusal as refusal

from build_system.tests.packaging.test_macos_binary_apply import inputs as inputs

ROOT = Path(__file__).resolve().parents[3]


def facts() -> tuple[dict, dict]:
    previous = {
        "artifact": {"version": "0.7.0", "sha256": "a" * 64},
        "source_commit": "1" * 40,
        "binaries": ["capsem", "capsem-service"],
    }
    candidate = {"artifact": {"version": "0.7.1", "sha256": "b" * 64}, "source_commit": "2" * 40}
    spec = {
        "before": previous,
        "candidate": candidate,
        "source": "http://127.0.0.1:18765/candidate/assets/stable/manifest.json",
        "channel": "stable",
    }
    snapshot = {
        "files": {
            name: [1, 2, 3, "a" * 64] for name in refusal.required_files(previous["binaries"])
        },
        "processes": {"service": "1 501 original-service", "gateway": "2 501 original-gateway"},
        "receipt": "version: 0.7.0",
        "root_stages": 0,
    }
    snapshot["files"]["archive/synthetic"] = [1, 2, 3, "a" * 64]
    installed = {
        "package_version": "0.7.0",
        "channel": "stable",
        "manifest_url": spec["source"],
        "package_receipt": True,
        "binary_cohort": True,
        "installed": True,
        "running": True,
        "service": "ok",
        "gateway": "ok",
    }
    report = {
        "schema": "capsem.release_glowup.v1",
        "adapter": refusal.ADAPTER,
        "artifact": previous["artifact"],
        "installed": installed,
        "capabilities": {"native_install": True, "launchd": True, "physical_vz_boot": False},
        "adapter_evidence": {
            "scope": refusal.SCOPE,
            "before": previous,
            "candidate": candidate,
            "installed_before": copy.deepcopy(installed),
            "installer_status": 1,
            "reason": "obsolete_host_hash_chain",
            "admission_phase": "admit_candidate_ledger",
            "snapshot_before": snapshot,
            "snapshot_after": copy.deepcopy(snapshot),
        },
    }
    return report, spec


def test_exact_refusal_preserves_before_without_apply_success() -> None:
    report, spec = facts()
    assert refusal.validate_report(report, spec) is report
    assert report["artifact"] != spec["candidate"]["artifact"]


@pytest.mark.parametrize(
    "damage",
    [
        "success",
        "retired",
        "data",
        "empty",
        "health",
        "stage",
        "candidate",
        "apply",
        "phase",
        "reason",
        "artifact",
    ],
)
def test_missing_or_foreign_refusal_evidence_cannot_pass(damage: str) -> None:
    report, spec = facts()
    evidence = report["adapter_evidence"]
    if damage == "success":
        evidence["installer_status"] = 0
    elif damage == "retired":
        evidence["snapshot_after"]["processes"]["service"] = "3 501 replacement"
    elif damage == "data":
        evidence["snapshot_after"]["files"]["host.db"][-1] = "b" * 64
    elif damage == "empty":
        evidence["snapshot_before"]["files"] = {}
    elif damage == "health":
        report["installed"]["service"] = "failed"
    elif damage == "stage":
        evidence["snapshot_before"]["root_stages"] = 1
    elif damage == "candidate":
        evidence["candidate"] = spec["before"]
    elif damage == "apply":
        evidence["scope"] = "binary_apply_only"
    elif damage == "phase":
        evidence["admission_phase"] = "remove_package_payload"
    elif damage == "reason":
        evidence["reason"] = "runtime_incompatible"
    else:
        report["artifact"] = spec["candidate"]["artifact"]
    with pytest.raises((ValueError, RuntimeError)):
        refusal.validate_report(report, spec)


def test_snapshot_hashes_original_bytes_and_rejects_changed_file(tmp_path: Path) -> None:
    path = tmp_path / "host.db"
    path.write_bytes(b"source-only original database bytes")
    original = refusal.file_identity(path)
    path.write_bytes(b"different database bytes")
    assert refusal.file_identity(path) != original
    path.unlink()
    path.symlink_to(tmp_path / "foreign")
    with pytest.raises(ValueError, match="regular"):
        refusal.file_identity(path)


def test_fifo_is_refused_before_any_blocking_read(tmp_path: Path) -> None:
    fifo = tmp_path / "host.db"
    os.mkfifo(fifo)
    with pytest.raises(ValueError, match="regular"):
        refusal.file_identity(fifo)


def test_empty_wal_coordination_is_not_confused_with_data_mutation() -> None:
    report, spec = facts()
    report["adapter_evidence"]["snapshot_after"]["files"]["host.db-wal"] = [
        1,
        9,
        0,
        refusal.crypto.sha256(b"").hexdigest(),
    ]
    assert refusal.validate_report(report, spec) is report
    report["adapter_evidence"]["snapshot_after"]["files"]["host.db-wal"][2] = 1
    with pytest.raises(ValueError, match="WAL"):
        refusal.validate_report(report, spec)


def admission_module(monkeypatch):
    monkeypatch.syspath_prepend(str(ROOT / "build_system/packaging/macos"))
    import macos_host_admission

    return macos_host_admission


@pytest.mark.parametrize("partial", ["package", "manifest_file", "sbom"])
def test_partial_candidate_options_refuse_before_any_staging(monkeypatch, partial):
    module = admission_module(monkeypatch)
    args = SimpleNamespace(**{f"admission_candidate_{partial}": Path("original")})
    with pytest.raises(ValueError, match="together"):
        module.enabled(args)


def test_separate_runtime_refusal_fixture_never_relaxes_binary_pair(inputs, tmp_path, monkeypatch):
    import json

    from capsem_builder.release.tools.release_macos_pairing import validate_pair

    before, after = inputs
    document = json.loads(after[1].read_text())
    document["runtime"]["revision"] = "genuine-separate-runtime"
    after[1].write_text(json.dumps(document))
    with pytest.raises(ValueError, match="identical verified runtime"):
        validate_pair(before=before, after=after, channel="nightly")
    module = admission_module(monkeypatch)
    args = SimpleNamespace(
        package=before[0],
        manifest_file=before[1],
        sbom=before[2],
        admission_candidate_package=after[0],
        admission_candidate_manifest_file=after[1],
        admission_candidate_sbom=after[2],
        channel="nightly",
        manifest_url="http://127.0.0.1:18765/candidate/assets/nightly/manifest.json",
    )
    share = tmp_path / "share"
    share.mkdir()
    spec = module.prepare(args, root=ROOT, share=share, guest_package=share / "before.pkg")
    assert spec["before"]["runtime_sha256"] != spec["candidate"]["runtime_sha256"]
    assert not (share / "binary-apply-pair.json").exists()
    args.before_package = before[0]
    with pytest.raises(ValueError, match="binary Apply"):
        module.enabled(args)


def test_guest_mode_original_installer_once_and_positive_old_health_order() -> None:
    source = (ROOT / "build_system/packaging/macos/macos-tart-host-admission.sh").read_text()
    installer = source.index("sudo /usr/sbin/installer")
    assert source.count("sudo /usr/sbin/installer") == 1
    assert source.index("capsem_host_admission snapshot") < installer
    assert (
        source.index('capsem_verify_host_before "$SHARE/host-refusal-before-installed.json"')
        < installer
    )
    assert (
        installer
        < source.index('capsem_verify_host_before "$SHARE/host-refusal-after-installed.json"')
        < source.index("capsem_host_admission finish")
    )
    assert "launchctl kickstart" not in source
    assert "capsem_pair_http request" not in source


def test_staged_refusal_module_uses_guest_python39_without_site(tmp_path, monkeypatch):
    admission_module(monkeypatch)
    from macos_candidate_content import stage_guest_scripts

    share = tmp_path / "share"
    stage_guest_scripts(ROOT, share)
    interpreter = "/usr/bin/python3" if Path("/usr/bin/python3").exists() else sys.executable
    result = subprocess.run(
        [
            interpreter,
            "-S",
            "-m",
            "capsem_builder.release.tools.release_admission_refusal",
            "--help",
        ],
        cwd=share,
        env={"PYTHONPATH": str(share)},
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 0, result.stderr
    original = share / "synthetic-original"
    original.write_bytes(b"synthetic original preservation bytes")
    preservation = subprocess.run(
        [
            interpreter,
            "-S",
            "-c",
            "from pathlib import Path; "
            "from capsem_builder.release.tools.release_admission_refusal import file_identity; "
            "assert len(file_identity(Path('synthetic-original'))[3]) == 64",
        ],
        cwd=share,
        env={"PYTHONPATH": str(share)},
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert preservation.returncode == 0, preservation.stderr


@pytest.mark.parametrize(
    "text",
    [
        "phase=admit_candidate_ledger runtime incompatible",
        "phase=admit_candidate_ledger obsolete host hash-chain schema event=remove_old_payload",
        "phase=stop_existing_install obsolete host hash-chain schema",
    ],
)
def test_wrong_refusal_or_retirement_log_never_produces_report(tmp_path: Path, text: str) -> None:
    home = tmp_path / "home/.capsem"
    (home / "logs").mkdir(parents=True)
    (home / "logs/install-latest.log").write_text(text)
    _, spec = facts()
    with pytest.raises(ValueError, match=r"HOST|retirement"):
        refusal.finish(tmp_path, home, spec, 4096)
    assert not (tmp_path / "report.json").exists()


def test_ordinary_mode_still_requires_all_original_transition_inputs(monkeypatch):
    module = admission_module(monkeypatch)
    with pytest.raises(ValueError, match="all transition"):
        module.enabled(SimpleNamespace())
    assert (
        module.enabled(
            SimpleNamespace(
                updated_manifest_file=Path("original-update"),
                tampered_manifest_file=Path("original-tamper"),
                incompatible_manifest_file=Path("original-incompatible"),
            )
        )
        is False
    )


def test_refusal_report_cannot_inherit_apply_or_physical_success() -> None:
    report, spec = facts()
    report["activation"] = "verified"
    with pytest.raises(ValueError, match="other proof"):
        refusal.validate_report(report, spec)
    del report["activation"]
    report["capabilities"]["physical_vz_boot"] = True
    with pytest.raises(ValueError, match="unproved"):
        refusal.validate_report(report, spec)
