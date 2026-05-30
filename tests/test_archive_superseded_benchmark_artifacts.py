import importlib.util
import json
import sys
import zipfile
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = PROJECT_ROOT / "scripts" / "archive_superseded_benchmark_artifacts.py"
SPEC = importlib.util.spec_from_file_location("archive_superseded_benchmark_artifacts", SCRIPT_PATH)
archive_script = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = archive_script
SPEC.loader.exec_module(archive_script)


def write_json(path: Path, data: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def test_archives_oldest_generated_artifact_per_lane(tmp_path):
    old = tmp_path / "benchmarks" / "capsem-bench" / "data_1.2.1_x86_64.json"
    new = tmp_path / "benchmarks" / "capsem-bench" / "data_1.2.2_x86_64.json"
    write_json(old, {"project_version": "1.2.1", "arch": "x86_64", "recorded_at": 10})
    write_json(new, {"project_version": "1.2.2", "arch": "x86_64", "recorded_at": 20})

    archive_path, archived = archive_script.archive_superseded(
        tmp_path,
        archive_name="history.zip",
    )

    assert archive_path == tmp_path / "benchmarks" / "archive" / "history.zip"
    assert [item.path for item in archived] == [old]
    assert not old.exists()
    assert new.exists()
    with zipfile.ZipFile(archive_path) as zf:
        assert "MANIFEST.json" in zf.namelist()
        assert "benchmarks/capsem-bench/data_1.2.1_x86_64.json" in zf.namelist()
        manifest = json.loads(zf.read("MANIFEST.json"))
    assert manifest["schema"] == "capsem.benchmark-archive.v1"
    assert manifest["artifacts"][0]["project_version"] == "1.2.1"
    assert manifest["artifacts"][0]["git_commit"] is None


def test_keeps_latest_per_security_engine_suffix(tmp_path):
    old_process = tmp_path / "benchmarks" / "security-engine" / "data_1.2.1_x86_64_process_enforcement.json"
    new_process = tmp_path / "benchmarks" / "security-engine" / "data_1.2.2_x86_64_process_enforcement.json"
    cel = tmp_path / "benchmarks" / "security-engine" / "data_1.2.1_x86_64_cel_microbench.json"
    write_json(old_process, {"project_version": "1.2.1", "arch": "x86_64", "recorded_at": 10})
    write_json(new_process, {"project_version": "1.2.2", "arch": "x86_64", "recorded_at": 20})
    write_json(cel, {"project_version": "1.2.1", "arch": "x86_64", "recorded_at": 10})

    _archive_path, archived = archive_script.archive_superseded(
        tmp_path,
        archive_name="history.zip",
    )

    assert [item.path for item in archived] == [old_process]
    assert new_process.exists()
    assert cel.exists()


def test_keeps_legacy_macos_lifecycle_until_arch_scoped_lane_exists(tmp_path):
    legacy = tmp_path / "benchmarks" / "lifecycle" / "data_1.2.1.json"
    linux = tmp_path / "benchmarks" / "lifecycle" / "data_1.2.1_x86_64.json"
    write_json(legacy, {"version": "0.1.0", "timestamp": 10})
    write_json(linux, {"project_version": "1.2.1", "arch": "x86_64", "recorded_at": 10})

    archive_path, archived = archive_script.archive_superseded(
        tmp_path,
        archive_name="history.zip",
    )

    assert archive_path is None
    assert archived == []
    assert legacy.exists()
    assert linux.exists()


def test_archives_legacy_macos_lifecycle_after_arch_scoped_lane_exists(tmp_path):
    legacy = tmp_path / "benchmarks" / "lifecycle" / "data_1.2.1.json"
    scoped = tmp_path / "benchmarks" / "lifecycle" / "data_1.2.2_arm64.json"
    write_json(legacy, {"version": "0.1.0", "timestamp": 10})
    write_json(scoped, {"project_version": "1.2.2", "arch": "arm64", "recorded_at": 20})

    _archive_path, archived = archive_script.archive_superseded(
        tmp_path,
        archive_name="history.zip",
    )

    assert [item.path for item in archived] == [legacy]
    assert not legacy.exists()
    assert scoped.exists()


def test_dry_run_does_not_delete_or_write_archive(tmp_path):
    old = tmp_path / "benchmarks" / "host-native" / "data_1.2.1_x86_64.json"
    new = tmp_path / "benchmarks" / "host-native" / "data_1.2.2_x86_64.json"
    write_json(old, {"project_version": "1.2.1", "arch": "x86_64", "recorded_at": 10})
    write_json(new, {"project_version": "1.2.2", "arch": "x86_64", "recorded_at": 20})

    archive_path, archived = archive_script.archive_superseded(
        tmp_path,
        archive_name="history.zip",
        dry_run=True,
    )

    assert archive_path == tmp_path / "benchmarks" / "archive" / "history.zip"
    assert [item.path for item in archived] == [old]
    assert old.exists()
    assert new.exists()
    assert not archive_path.exists()
