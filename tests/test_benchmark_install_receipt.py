"""Only exact owning proof joins can become normal Install raw observations."""

import json

import pytest
from benchmark_install_fixture import linux_receipt, load, prepared

from benchmarks.helpers.install_receipt import sidecar, verified_receipt


def test_genuine_shape_and_failed_original_remain_separate(tmp_path, monkeypatch):
    path, inputs = prepared(tmp_path, monkeypatch, platform="linux")
    selected = load(path, inputs)
    native = tmp_path / "native"
    linux_receipt(native, selected)
    result = verified_receipt(native, selected, maximum=1048576)
    assert result["activation"] == "verified"
    assert result["original_request"]["outcome"] == "transport_failure"
    stored = json.loads(sidecar(selected, result))
    assert stored["before"]["package_sha256"] != stored["after"]["package_sha256"]
    assert stored["after"]["source_commit"] == "b" * 40


@pytest.mark.parametrize(
    "damage",
    [
        "marker",
        "original_file",
        "transition_file",
        "missing_doctor",
        "wrong_predecessor",
        "wrong_source",
        "automatic_winner",
    ],
)
def test_stale_missing_or_foreign_native_join_cannot_be_a_sample(tmp_path, monkeypatch, damage):
    path, inputs = prepared(tmp_path, monkeypatch, platform="linux")
    selected = load(path, inputs)
    native = tmp_path / "native"
    report = linux_receipt(native, selected)
    if damage == "marker":
        (native / "candidate-after-audit-line").write_text("5\n")
    elif damage in {"original_file", "transition_file"}:
        filename = (
            "candidate-apply-request.json"
            if damage == "original_file"
            else "candidate-after-transition.json"
        )
        (native / filename).write_text("{}")
    elif damage == "missing_doctor":
        (native / "candidate-after-doctor.json").unlink()
    elif damage == "wrong_predecessor":
        report["adapter_evidence"]["release_pairing"]["before"]["package_sha256"] = "f" * 64
    else:
        receipt = report["adapter_evidence"]["explicit_apply"]
        receipt["original_request"][
            "attempts" if damage == "automatic_winner" else "source_commit"
        ] = 0 if damage == "automatic_winner" else "f" * 40
        (native / "candidate-apply.json").write_text(json.dumps(receipt))
    (native / "report.json").write_text(json.dumps(report))
    with pytest.raises((ValueError, RuntimeError, FileNotFoundError)):
        verified_receipt(native, selected, maximum=1048576)


@pytest.mark.parametrize("damage", [None, "retirement", "installed", "version"])
def test_mac_original_owner_files_remain_bound_to_the_observation(tmp_path, monkeypatch, damage):
    from capsem_builder.release.tools.release_macos_apply import finish

    from build_system.tests.packaging.test_macos_binary_apply_evidence import evidence

    native = tmp_path / "native"
    share = native / "share"
    share.mkdir(parents=True)
    path, inputs = prepared(share, monkeypatch)
    selected = load(path, inputs)
    files = tuple(
        (phase.package, phase.manifest, phase.sbom)
        for phase in (selected.inputs.before, selected.inputs.after)
    )
    spec = evidence(share, files)
    (share / "binary-apply-pair.json").write_text(json.dumps(spec))
    report = finish(share, spec, source=selected.inputs.tart.manifest_url, maximum=1048576)
    (native / "report.json").write_text(json.dumps(report))
    if damage:
        filename = {
            "retirement": "binary-apply-retirement.json",
            "installed": "binary-apply-after-installed.json",
            "version": "binary-apply-after-version.txt",
        }[damage]
        (share / filename).write_text("{}")
        with pytest.raises((ValueError, RuntimeError, KeyError)):
            verified_receipt(native, selected, maximum=1048576)
    else:
        checked = verified_receipt(native, selected, maximum=1048576)
        assert checked["original_request"]["outcome"] == "transport_failure"
        assert checked["capabilities"]["physical_vz_boot"] is False
