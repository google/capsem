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


@pytest.mark.parametrize("foreign", [False, True])
def test_default_bin_symlink_matches_only_the_current_cargo_files(tmp_path, foreign):
    config = load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    selected = config.path(config.modules.default_bin_dir)
    shared = tmp_path / "shared-cargo"
    shared.mkdir()
    selected.parent.mkdir(parents=True, exist_ok=True)
    selected.symlink_to(shared, target_is_directory=True)
    rows = []
    for name in config.signing.built:
        (shared / name).write_bytes(b"same synthetic executable")
        entry = row(tmp_path, tests=False, features=())
        entry["target"]["name"] = name
        entry["executable"] = str(shared / name)
        rows.append(entry)
    if foreign:
        other = tmp_path / "foreign-build"
        other.mkdir()
        (other / config.signing.built[0]).write_bytes(b"same synthetic executable")
        rows[0]["executable"] = str(other / config.signing.built[0])
    log = config.path(config.functional.native_file_content.host_log)
    log.parent.mkdir(parents=True, exist_ok=True)
    write(log, [*rows, {"reason": "build-finished", "success": True}])
    if foreign:
        with pytest.raises(ValueError, match="not produced"):
            verify_host_roles(config, selected, None)
    else:
        verify_host_roles(config, selected, None)


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


@pytest.fixture
def assembly(tmp_path, monkeypatch):
    from capsem_builder import crypto
    from capsem_builder.cache.config import load_policy
    from capsem_builder.cache.paths import CachePaths
    from capsem_builder.gate.tools import native_file_content as owner

    config = load(PROJECT_ROOT).model_copy(update={"root": tmp_path})
    settings = config.functional.native_file_content
    bins = config.path(config.modules.default_bin_dir)
    bins.mkdir(parents=True)
    rows = []
    sources = {}
    for name in config.signing.built:
        path = bins / name
        path.write_bytes(b"synthetic executable " + name.encode())
        path.chmod(0o555)
        sources[name] = path
        entry = row(tmp_path, tests=False, features=())
        entry["target"]["name"] = name
        entry["executable"] = str(path)
        rows.append(entry)
    feature = tmp_path / "cache/target/capsem-service-test"
    feature.write_bytes(b"synthetic feature executable")
    feature.chmod(0o555)
    sources[settings.artifact_name] = feature
    config.path(settings.host_log).parent.mkdir(parents=True, exist_ok=True)
    write(config.path(settings.host_log), [*rows, {"reason": "build-finished", "success": True}])
    write(
        config.path(settings.cargo_log),
        [row(tmp_path), {"reason": "build-finished", "success": True}],
    )
    identity = {"source_commit": "synthetic-source", "source_tree": "synthetic-tree"}
    config.path(settings.source_receipt).write_text(json.dumps(identity))
    monkeypatch.setattr(owner, "source_identity", lambda _: identity)
    policy = load_policy(PROJECT_ROOT)
    paths = CachePaths(repository_root=tmp_path, policy=policy)
    monkeypatch.setattr(owner, "load_paths", lambda _: paths)
    receipt = {
        "schema": "capsem.native-file-content.v1",
        **identity,
        "binaries": {name: crypto.digest_file(path) for name, path in sources.items()},
        "cohort_package": None,
    }
    encoded = json.dumps(receipt, sort_keys=True).encode()
    key = settings.generation_prefix + crypto.blake3(encoded).hexdigest()
    directory = paths.stage(settings.cache_stage) / key
    expected = sum(path.stat().st_size for path in sources.values()) + len(encoded)
    expected += sum(
        config.path(log).stat().st_size for log in (settings.cargo_log, settings.host_log)
    )

    def limit(**values):
        nonlocal paths
        stages = dict(policy.stages)
        stages[settings.cache_stage] = stages[settings.cache_stage].model_copy(update=values)
        paths = paths.model_copy(update={"policy": policy.model_copy(update={"stages": stages})})

    return config, bins, paths, directory, expected, limit


@pytest.mark.parametrize("pressure", ["entries", "bytes"])
def test_protected_capacity_refuses_before_any_native_generation_write(assembly, pressure):
    from capsem_builder.cache.leases import release_path, retain_generation
    from capsem_builder.gate.tools.native_file_content import assemble

    config, bins, paths, directory, expected, limit = assembly
    stage = config.functional.native_file_content.cache_stage
    limit(
        max_size_bytes=expected if pressure == "bytes" else 100000,
        warm_size_bytes=0,
        maximum_count=4,
    )
    leases = []
    try:
        for index in range(4):
            key = f"foreign-{index}"
            foreign = paths.stage(stage) / key
            foreign.mkdir(parents=True)
            (foreign / "original").write_bytes(b"held")
            retain_generation(paths, stage, key)
            leases.append(foreign.parent / f".{key}.lock")
        with pytest.raises(ValueError, match=r"native.*capacity"):
            assemble(config, bins, None)
        assert not directory.exists(), "admission must precede generation mkdir and every copy"
        assert not config.path(config.functional.native_file_content.selection).exists()
        assert all(
            (paths.stage(stage) / f"foreign-{i}/original").read_bytes() == b"held" for i in range(4)
        )
    finally:
        for lease in leases:
            release_path(lease)


def test_exact_byte_admission_and_equal_generation_reuse(assembly, monkeypatch):
    from capsem_builder.gate.tools import native_file_content as owner

    config, bins, _, directory, expected, limit = assembly
    limit(max_size_bytes=expected, warm_size_bytes=0, maximum_count=1)
    assert owner.assemble(config, bins, None) == directory
    assert sum(path.stat().st_size for path in directory.iterdir()) == expected
    before = {
        path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in directory.iterdir()
    }
    monkeypatch.setattr(owner.shutil, "copy2", lambda *_: pytest.fail("equal files must be reused"))
    assert owner.assemble(config, bins, None) == directory
    assert before == {
        path.name: (path.read_bytes(), path.stat().st_mtime_ns) for path in directory.iterdir()
    }


def test_wrong_existing_role_refused_before_missing_roles_are_copied(assembly):
    from capsem_builder.gate.tools.native_file_content import assemble

    config, bins, _, directory, _, _ = assembly
    directory.mkdir(parents=True)
    wrong = directory / config.functional.native_file_content.artifact_name
    wrong.write_bytes(b"foreign executable")
    with pytest.raises(ValueError, match="generation changed"):
        assemble(config, bins, None)
    assert list(directory.iterdir()) == [wrong]
    assert wrong.read_bytes() == b"foreign executable"


@pytest.mark.parametrize("fits", [False, True])
def test_partial_generation_projects_missing_receipt_role_and_log_growth(
    assembly, monkeypatch, fits
):
    from capsem_builder.gate.tools import native_file_content as owner

    config, bins, _, directory, expected, limit = assembly
    owner.assemble(config, bins, None)
    settings = config.functional.native_file_content
    removed = directory / config.signing.built[0]
    removed_bytes = removed.stat().st_size
    removed.unlink()
    receipt = directory / settings.receipt_name
    receipt_bytes = receipt.stat().st_size
    receipt.unlink()
    log = config.path(settings.host_log)
    previous = log.stat().st_size
    rows = [json.loads(line) for line in log.read_text().splitlines()]
    write(log, [*rows[:-1], {"reason": "compiler-message", "message": "new diagnostic"}, rows[-1]])
    growth = log.stat().st_size - previous
    limit(max_size_bytes=expected + growth - (0 if fits else 1), warm_size_bytes=0, maximum_count=1)
    enforce = owner.enforce_repository
    intents = []

    def admit(*args, **kwargs):
        intents.append(kwargs["required"])
        return enforce(*args, **kwargs)

    monkeypatch.setattr(owner, "enforce_repository", admit)
    before = {path.name: path.read_bytes() for path in directory.iterdir()}
    if fits:
        owner.assemble(config, bins, None)
        assert sum(path.stat().st_size for path in directory.iterdir()) == expected + growth
    else:
        with pytest.raises(ValueError, match=r"native.*capacity"):
            owner.assemble(config, bins, None)
        assert before == {path.name: path.read_bytes() for path in directory.iterdir()}
    assert len(intents) == 1
    assert intents[0].entries == 0
    assert intents[0].bytes == removed_bytes + receipt_bytes + growth
