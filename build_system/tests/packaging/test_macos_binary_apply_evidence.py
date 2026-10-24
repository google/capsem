"""Scoped Tart binary activation never inherits original HTTP or VZ success."""

from __future__ import annotations

import json
import time
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.release.tools import release_macos_apply, release_macos_pairing

from build_system.tests.packaging.test_macos_binary_apply import inputs as inputs

SOURCE = "http://127.0.0.1:18765/candidate/assets/nightly/manifest.json"


def evidence(root: Path, raw_inputs):
    before, after = raw_inputs
    pair = release_macos_pairing.validate_pair(before=before, after=after, channel="nightly")
    pair["inputs"] = {
        phase: {
            name: path.relative_to(root).as_posix()
            for name, path in zip(("package", "manifest", "sbom"), files, strict=True)
        }
        for phase, files in (("before", before), ("after", after))
    }
    pair["limits"] = {"timeout": 0.5, "maximum": 16384}
    request = {
        "schema": "capsem.installed_apply_request.v1",
        "source": SOURCE,
        "source_commit": pair["after"]["source_commit"],
        "candidate_manifest_sha256": pair["after"]["manifest_sha256"],
        "after_line": 0,
        "attempts": 1,
        "outcome": "transport_failure",
        "transport": "service_uds",
        "error_type": "ConnectionError",
        "started_ns": time.monotonic_ns(),
        "elapsed_ns": 0,
    }
    verdict = {
        "schema": "capsem.release_transition_verdict.v1",
        "kind": "binary_only",
        "result": "activated",
        "source": SOURCE,
        "candidate_manifest_sha256": pair["after"]["manifest_sha256"],
        "installed_manifest_sha256": pair["after"]["manifest_sha256"],
        "fetched": True,
        "preserved_previous": False,
    }
    documents = {
        "binary-apply-request": request,
        "binary-apply-transition": verdict,
        "binary-apply-retirement": {
            "old_service_pid": 111,
            "new_service_pid": 222,
            "old_service_retired": True,
        },
    }
    for phase in ("before", "after"):
        documents[f"binary-apply-{phase}-installed"] = {
            "package_version": pair[phase]["artifact"]["version"],
            "channel": "nightly",
            "manifest_url": SOURCE,
            "metadata_manifest_url": pair[phase]["metadata_manifest_url"],
            "installed": True,
            "running": True,
            "service": "ok",
            "gateway": "ok",
        }
        (root / f"binary-apply-{phase}-version.txt").write_text(
            f"capsem {pair[phase]['artifact']['version']} (build {pair[phase]['source_commit'][:12]}.123 ts=test)\n"
        )
    for name, value in documents.items():
        (root / f"{name}.json").write_text(json.dumps(value))
    (root / "binary-apply-audit-line").write_text("0\n")
    return pair


def test_original_eof_and_native_pair_activation_remain_separate(inputs, tmp_path):
    pair = evidence(tmp_path, inputs)
    report = release_macos_apply.finish(tmp_path, pair, source=SOURCE, maximum=16384)
    assert report["activation"] == "verified"
    assert report["original_request"]["outcome"] == "transport_failure"
    assert report["scope"] == "binary_apply_only"
    assert all(
        report["capabilities"][field] is False
        for field in ("physical_vz_boot", "full_doctor", "installed_winterfell")
    )


@pytest.mark.parametrize(
    "damage", ["old_alive", "same_pid", "source", "marker", "installed", "missing_audit"]
)
def test_guest_pair_receipt_rejects_unproven_original_or_native_phase(inputs, tmp_path, damage):
    pair = evidence(tmp_path, inputs)
    if damage == "source":
        (tmp_path / "binary-apply-after-version.txt").write_text(
            "capsem after (build cccccccccccc.123)\n"
        )
    elif damage == "marker":
        (tmp_path / "binary-apply-audit-line").write_text("1\n")
    elif damage == "missing_audit":
        (tmp_path / "binary-apply-transition.json").unlink()
    else:
        name = (
            "binary-apply-after-installed" if damage == "installed" else "binary-apply-retirement"
        )
        path = tmp_path / f"{name}.json"
        value = json.loads(path.read_text())
        value.update(
            {
                "old_alive": {"old_service_retired": False},
                "same_pid": {"new_service_pid": 111},
                "installed": {"package_version": "foreign"},
            }[damage]
        )
        path.write_text(json.dumps(value))
    with pytest.raises((ValueError, FileNotFoundError)):
        release_macos_apply.finish(tmp_path, pair, source=SOURCE, maximum=16384)


@pytest.mark.parametrize("damage", ["old_alive", "installed", "physical"])
def test_host_cannot_promote_false_native_or_physical_report(inputs, tmp_path, monkeypatch, damage):
    pair = evidence(tmp_path, inputs)
    report = release_macos_apply.finish(tmp_path, pair, source=SOURCE, maximum=16384)
    if damage == "old_alive":
        report["retirement"]["old_service_retired"] = False
    elif damage == "installed":
        report["installed"]["after"]["package_version"] = "foreign"
    else:
        report["capabilities"]["physical_vz_boot"] = True
    path = tmp_path / "report.json"
    path.write_text(json.dumps(report))
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    with pytest.raises(ValueError):
        macos_binary_apply.validate_report(path, pair, source=SOURCE)


def test_second_mode_reuses_existing_tart_command_and_distinct_owned_workdir(
    inputs, tmp_path, monkeypatch
):
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    before, _after = inputs
    args = SimpleNamespace(
        before_package=before[0], before_manifest_file=before[1], before_sbom=before[2]
    )
    command = ["python", "existing-macos_tart_glowup.py", "--work-dir", str(tmp_path / "normal")]
    issued = []
    result = macos_binary_apply.run_paired(args, command, runner=issued.append)
    assert result == tmp_path / "normal/binary-apply/report.json"
    assert len(issued) == 1
    assert command[-1] == str(tmp_path / "normal")
    assert issued[0][1] == "existing-macos_tart_glowup.py"
    assert issued[0][issued[0].index("--before-package") + 1] == str(before[0])
