"""Exact two-package metadata and real payload bytes, without installation."""

from __future__ import annotations

import gzip
import json
import struct
import subprocess
import sys
import zlib
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder import crypto
from capsem_builder.release.tools import package_payload


def write_pkg(path, contents):
    archive = bytearray()
    for name, data in [*contents.items(), ("TRAILER!!!", b"")]:
        encoded = name.encode() + b"\0"
        fields = [1, 0o100755, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(encoded), 0]
        archive.extend(b"070701" + b"".join(f"{value:08x}".encode() for value in fields))
        archive.extend(encoded)
        archive.extend(b"\0" * (-len(archive) % 4))
        archive.extend(data)
        archive.extend(b"\0" * (-len(archive) % 4))
    payload = gzip.compress(archive)
    toc = f"<xar><toc><file><name>Payload</name><data><offset>0</offset><length>{len(payload)}</length></data></file></toc></xar>".encode()
    compressed = zlib.compress(toc)
    path.write_bytes(
        struct.pack(">4sHHQQI", b"xar!", 28, 1, len(compressed), len(toc), 0) + compressed + payload
    )


def cohort(root: Path, label: str):
    version = "0.7.0" if label == "before" else "0.7.1"
    directory = root / label
    directory.mkdir()
    package = directory / f"Capsem-{label}.pkg"
    files = {
        f"usr/local/bin/{name}": f"{label}-{name}".encode() for name in ("capsem", "capsem-service")
    }
    files["usr/local/share/capsem/assets/manifest-metadata.json"] = json.dumps(
        {
            "schema": "capsem.manifest_metadata.v1",
            "channel": "nightly",
            "manifest_url": "http://127.0.0.1:18765/candidate/assets/nightly/manifest.json",
        }
    ).encode()
    write_pkg(package, files)
    binary_rows, spdx = [], []
    for path, data in files.items():
        name = Path(path).name
        if not name.startswith("capsem"):
            continue
        digest = crypto.sha256(data).hexdigest()
        reference = f"SPDXRef-File-{name}"
        binary_rows.append(
            {
                "name": name,
                "version": version,
                "platform": "macos",
                "architecture": "arm64",
                "status": "current",
                "installed_path": f"/{path}",
                "bytes": len(data),
                "digest": {"sha256": digest},
                "sbom_component_ref": reference,
            }
        )
        spdx.append(
            {
                "SPDXID": reference,
                "fileName": f"/{path}",
                "checksums": [{"algorithm": "SHA256", "checksumValue": digest}],
            }
        )
    sbom = directory / "capsem-sbom.spdx.json"
    sbom.write_text(json.dumps({"spdxVersion": "SPDX-2.3", "files": spdx}))
    row = {
        "name": package.name,
        "version": version,
        "platform": "macos",
        "architecture": "arm64",
        "bytes": package.stat().st_size,
        "digest": {"sha256": crypto.sha256(package.read_bytes()).hexdigest()},
        "source_commit": ("a" if label == "before" else "b") * 40,
        "status": "current",
        "binaries": binary_rows,
        "evidence": [
            {
                "kind": "sbom",
                "status": "current",
                "bytes": sbom.stat().st_size,
                "digest": {"sha256": crypto.sha256(sbom.read_bytes()).hexdigest()},
            }
        ],
    }
    manifest = directory / "manifest.json"
    manifest.write_text(
        json.dumps(
            {
                "channel": "nightly",
                "packages": [row],
                "runtime": {"revision": "same-real-runtime", "architectures": []},
            }
        )
    )
    return package, manifest, sbom


@pytest.fixture
def inputs(tmp_path, monkeypatch):
    # Exercise the real portable archive decoder, not an installer executable.
    monkeypatch.setattr(package_payload.shutil, "which", lambda _tool: None)
    return cohort(tmp_path, "before"), cohort(tmp_path, "after")


def pairing_module():
    from capsem_builder.release.tools import release_macos_pairing

    return release_macos_pairing


def test_pairing_joins_distinct_exact_package_source_sbom_and_real_payload(inputs):
    before, after = inputs
    result = pairing_module().validate_pair(before=before, after=after, channel="nightly")
    assert result["before"]["artifact"]["sha256"] != result["after"]["artifact"]["sha256"]
    assert result["before"]["source_commit"] == "a" * 40
    assert result["after"]["source_commit"] == "b" * 40
    assert result["before"]["runtime_sha256"] == result["after"]["runtime_sha256"]
    assert result["scope"] == "binary_apply_only"
    assert (
        result["before"]["metadata_manifest_url"]
        == "http://127.0.0.1:18765/candidate/assets/nightly/manifest.json"
    )


@pytest.mark.parametrize(
    "damage",
    ["runtime", "source", "sbom", "payload", "reference", "same", "revoked", "binary_version"],
)
def test_pairing_refuses_foreign_metadata_payload_or_runtime(inputs, damage):
    before, after = inputs
    if damage == "same":
        after = before
    elif damage == "sbom":
        after[2].write_text("{}")
    else:
        document = json.loads(after[1].read_text())
        row = document["packages"][0]
        if damage == "runtime":
            document["runtime"]["revision"] = "different"
        elif damage == "source":
            row["source_commit"] = "guessed"
        elif damage == "payload":
            row["binaries"][0]["digest"]["sha256"] = "c" * 64
        elif damage == "reference":
            row["binaries"][0]["sbom_component_ref"] = "missing"
        elif damage == "revoked":
            row["binaries"][0]["status"] = "revoked"
        elif damage == "binary_version":
            row["binaries"][0]["version"] = "foreign"
        after[1].write_text(json.dumps(document))
    with pytest.raises(ValueError):
        pairing_module().validate_pair(before=before, after=after, channel="nightly")


@pytest.mark.parametrize(
    "module",
    [
        "helpers.installed_apply_probe",
        "capsem_builder.release.tools.release_macos_apply",
        "capsem_builder.release.tools.release_macos_public_upgrade",
    ],
)
def test_staged_original_apply_helper_needs_only_standard_library(tmp_path, monkeypatch, module):
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    from macos_candidate_content import stage_guest_scripts

    share = tmp_path / "share"
    stage_guest_scripts(root, share)
    result = subprocess.run(
        [sys.executable, "-S", "-m", module, "--help"],
        cwd=share,
        env={"PYTHONPATH": str(share)},
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "usage:" in result.stdout
    if module == "helpers.installed_apply_probe":
        assert "--gateway-run-dir" in result.stdout


def test_existing_tart_owner_stages_pair_as_separate_mode(inputs, tmp_path, monkeypatch):
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    before, after = inputs
    args = SimpleNamespace(
        before_package=before[0],
        before_manifest_file=before[1],
        before_sbom=before[2],
        package=after[0],
        manifest_file=after[1],
        sbom=after[2],
        channel="nightly",
        manifest_url="http://127.0.0.1:18765/candidate/assets/nightly/manifest.json",
    )
    share = tmp_path / "share"
    share.mkdir()
    guest_package = share / "candidate" / after[0].name
    guest_package.parent.mkdir()
    guest_package.write_bytes(after[0].read_bytes())
    result = macos_binary_apply.prepare(args, root=root, share=share, guest_package=guest_package)
    assert result is not None
    assert result["scope"] == "binary_apply_only"
    assert (share / "binary-apply-pair.json").is_file()
    assert (share / "candidate/assets/nightly/manifest.json").read_bytes() == before[1].read_bytes()
    assert result["limits"]["timeout"] == 30
    assert result["before"]["source_commit"] != result["after"]["source_commit"]


def test_partial_before_pair_is_refused_before_any_staging(tmp_path, monkeypatch):
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    args = SimpleNamespace(
        before_package=tmp_path / "p", before_manifest_file=None, before_sbom=None
    )
    with pytest.raises(ValueError, match="together"):
        macos_binary_apply.prepare(args, root=root, share=tmp_path, guest_package=tmp_path / "p")
    assert list(tmp_path.iterdir()) == []


def test_paired_mode_uses_same_tart_owner_and_never_kickstarts_after_apply():
    root = Path(__file__).resolve().parents[3]
    main = (root / "build_system/packaging/macos/macos_tart_guest.sh").read_text()
    assert main.index("capsem_prove_binary_apply") < main.index(
        "=== Installing exact shared package ==="
    )
    source = (root / "build_system/packaging/macos/macos-tart-binary-apply.sh").read_text()
    assert source.count("capsem_pair_http request") == 1
    assert 'transport=(--gateway-run-dir "$CAPSEM_HOME/run")' in source
    after_request = source.split("capsem_pair_http request", 1)[1]
    assert "launchctl kickstart" not in after_request
    assert 'capsem_wait_for_service "$old_service_pid"' in after_request
    assert "binary_only activated" in after_request
    assert "capsem_pair_verify after" in after_request


@pytest.mark.parametrize("version", ["0.7.0", "0.6.9", "invalid", "01.7.1"])
def test_pair_refuses_versions_that_cannot_trigger_original_apply(inputs, version):
    before, after = inputs
    document = json.loads(after[1].read_text())
    document["packages"][0]["version"] = version
    for binary in document["packages"][0]["binaries"]:
        binary["version"] = version
    after[1].write_text(json.dumps(document))
    with pytest.raises(ValueError, match="newer stable semver"):
        pairing_module().validate_pair(before=before, after=after, channel="nightly")
