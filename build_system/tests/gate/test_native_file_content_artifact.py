"""A native test artifact must not borrow a stale/foreign Cargo generation."""

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
from capsem_builder.gate.config import load
from capsem_builder.gate.tools.native_file_content import artifacts, verify_host_roles
from helpers.gate import PROJECT_ROOT


def test_declared_native_command_bootstraps_without_repository_pythonpath():
    environment = dict(os.environ)
    environment.pop("PYTHONPATH", None)
    result = subprocess.run(
        [sys.executable, "build_system/scripts/test/native-file-content-artifact.py", "--help"],
        cwd=PROJECT_ROOT,
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, result.stderr
    assert "begin" in result.stdout and "assemble" in result.stdout


def row(root: Path, *, tests=True, features=("native-acceptance",)) -> dict[str, Any]:
    return {
        "reason": "compiler-artifact",
        "target": {"name": "capsem-service", "kind": ["bin"]},
        "profile": {"test": tests},
        "features": features,
        "manifest_path": str(root / "crates/capsem-service/Cargo.toml"),
        "executable": str(root / "cache/target/capsem-service-test"),
    }


def write(path, rows):
    path.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
    return path


def test_exact_successful_feature_artifact_is_selected(tmp_path):
    log = write(
        tmp_path / "cargo.jsonl", [row(tmp_path), {"reason": "build-finished", "success": True}]
    )
    assert artifacts(log, tmp_path, tests=True) == {
        "capsem-service": tmp_path / "cache/target/capsem-service-test"
    }


@pytest.mark.parametrize("defect", ["foreign-source", "not-test", "no-feature", "failed", "stale"])
def test_foreign_failed_or_stale_native_artifact_is_refused(tmp_path, defect):
    artifact = row(tmp_path)
    rows: list[dict[str, Any]] = [artifact, {"reason": "build-finished", "success": True}]
    if defect == "foreign-source":
        artifact["manifest_path"] = "/foreign/crates/capsem-service/Cargo.toml"
    elif defect == "not-test":
        artifact["profile"] = {"test": False}
    elif defect == "no-feature":
        artifact["features"] = []
    elif defect == "failed":
        rows[-1]["success"] = False
    else:
        rows.extend([{"reason": "compiler-message"}, {"reason": "build-finished", "success": True}])
    log = write(tmp_path / "cargo.jsonl", rows)
    with pytest.raises(ValueError):
        artifacts(log, tmp_path, tests=True)


def test_default_host_roles_require_the_complete_current_cargo_producer(tmp_path):
    config = load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    binary_dir = config.path(config.modules.default_bin_dir)
    rows = []
    for name in config.signing.built:
        artifact = row(tmp_path, tests=False, features=())
        artifact["target"]["name"] = name
        artifact["executable"] = str(binary_dir / name)
        rows.append(artifact)
    log = config.path(config.functional.native_file_content.host_log)
    log.parent.mkdir(parents=True)
    write(log, [*rows, {"reason": "build-finished", "success": True}])
    verify_host_roles(config, binary_dir, None)
    write(log, [*rows[:-1], {"reason": "build-finished", "success": True}])
    with pytest.raises(ValueError, match="not produced"):
        verify_host_roles(config, binary_dir, None)


def test_arbitrary_custom_host_directory_does_not_gain_current_source_receipt(
    tmp_path, monkeypatch
):
    config = load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    monkeypatch.delenv("CAPSEM_LOCAL_TEST_BINARY_COHORT", raising=False)
    with pytest.raises(ValueError, match="frozen source provenance"):
        verify_host_roles(config, tmp_path / "arbitrary-old-host", None)


def test_pulled_native_roles_must_equal_selected_package_payload_bytes(tmp_path, monkeypatch):
    from capsem_builder.gate.tools import native_file_content

    config = load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    binary_dir = tmp_path / "selected-package"
    binary_dir.mkdir()
    payload = {}
    for name in config.signing.built:
        data = f"synthetic unit payload for {name}".encode()
        payload[f"/usr/bin/{name}"] = data
        (binary_dir / name).write_bytes(data)
    monkeypatch.setattr(native_file_content, "package_payload_files", lambda _: payload)
    verify_host_roles(config, binary_dir, tmp_path / "unit-package.deb")
    (binary_dir / "capsem-process").write_bytes(b"different selected bytes")
    with pytest.raises(ValueError, match="differs from selected package"):
        verify_host_roles(config, binary_dir, tmp_path / "unit-package.deb")
