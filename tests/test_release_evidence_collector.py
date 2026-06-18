from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = PROJECT_ROOT / "scripts" / "release_collect_evidence.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("release_collect_evidence", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_release_evidence_collector_writes_honest_bundle(tmp_path):
    module = _load_module()
    root = tmp_path / "repo"
    root.mkdir()
    (root / "benchmarks" / "mock-server-protocol").mkdir(parents=True)
    (root / "benchmarks" / "mock-server-protocol" / "data.json").write_text(
        json.dumps(
            {
                "mock_server_protocol": {
                    "scenarios": [
                        {
                            "name": "model_json_response",
                            "total_requests": 50_000,
                            "concurrency": 64,
                            "successful": 50_000,
                            "failed": 0,
                            "requests_per_sec": 2974.3,
                            "latency_ms": {
                                "min": 0.7,
                                "max": 100.3,
                                "mean": 21.1,
                                "p50": 19.0,
                                "p95": 43.5,
                                "p99": 58.0,
                            },
                        }
                    ],
                    "websocket": [
                        {
                            "name": "websocket_echo",
                            "frames": 10,
                            "failed": False,
                            "frames_per_sec": 2173.5,
                            "latency_ms": {"p50": 0.2, "p95": 0.2, "p99": 0.2},
                        }
                    ],
                }
            }
        )
    )
    (root / "assets").mkdir()
    (root / "assets" / "manifest.json").write_text(
        json.dumps(
            {
                "format": 2,
                "assets": {"current": "2026.test"},
                "binaries": {"current": "1.3.test"},
                "refresh_policy": "24h",
            }
        )
    )
    (root / "CHANGELOG.md").write_text("## [Unreleased]\n")

    def fake_run(args, cwd):
        del cwd
        command = " ".join(args)
        if args[:2] == ["git", "status"]:
            return module.CommandResult(command=command, returncode=0, stdout="", stderr="")
        if args[:2] == ["git", "rev-parse"] and args[-1] == "HEAD":
            return module.CommandResult(command=command, returncode=0, stdout="abc123\n", stderr="")
        if args[:2] == ["git", "branch"]:
            return module.CommandResult(
                command=command, returncode=0, stdout="release/test\n", stderr=""
            )
        return module.CommandResult(command=command, returncode=1, stdout="", stderr="unexpected")

    bundle = module.collect_evidence(
        project_root=root,
        output_root=tmp_path / "release",
        timestamp="20260618T120000Z",
        run_command=fake_run,
    )

    manifest = json.loads((bundle / "manifest.json").read_text())
    assert manifest["schema"] == "capsem.release_evidence.v1"
    assert manifest["status"] == "non_manual_green_manual_pending"
    assert manifest["git"]["commit"] == "abc123"
    assert manifest["git"]["dirty"] is False
    assert manifest["manifest"]["format"] == 2
    assert manifest["manifest"]["current_binary"] == "1.3.test"
    assert manifest["manifest"]["current_assets"] == "2026.test"
    assert manifest["ironbank_guard"]["disabled_test_findings"] == []
    assert manifest["benchmark_summaries"][0]["sample_count"] == 50_000
    assert manifest["benchmark_summaries"][0]["concurrency"] == 64
    assert manifest["manual_gates_pending"]

    summary = (bundle / "benchmark-summary.md").read_text()
    assert "model_json_response" in summary
    assert "2974.3" in summary
    assert "websocket_echo" in summary

    manual = (bundle / "manual-gates-pending.md").read_text()
    assert "macOS package install" in manual
    assert "AGY" in manual


def test_release_evidence_collector_rejects_disabled_ironbank_tests(tmp_path):
    module = _load_module()
    root = tmp_path / "repo"
    root.mkdir()
    (root / "benchmarks").mkdir()
    (root / "assets").mkdir()
    (root / "assets" / "manifest.json").write_text("{}")
    (root / "CHANGELOG.md").write_text("## [Unreleased]\n")
    ironbank = root / "tests" / "ironbank"
    ironbank.mkdir(parents=True)
    (ironbank / "test_bad.py").write_text(
        "import pytest\n"
        "@pytest.mark.skip(reason='nope')\n"
        "def test_skip_marker():\n"
        "    pass\n"
        "def test_runtime_skip():\n"
        "    pytest.skip('nope')\n"
    )

    def fake_run(args, cwd):
        del cwd
        command = " ".join(args)
        if args[:2] == ["git", "status"]:
            return module.CommandResult(command=command, returncode=0, stdout="", stderr="")
        if args[:2] == ["git", "rev-parse"] and args[-1] == "HEAD":
            return module.CommandResult(command=command, returncode=0, stdout="abc123\n", stderr="")
        if args[:2] == ["git", "branch"]:
            return module.CommandResult(
                command=command, returncode=0, stdout="release/test\n", stderr=""
            )
        return module.CommandResult(command=command, returncode=1, stdout="", stderr="unexpected")

    with pytest.raises(RuntimeError, match="Ironbank disabled-test guard failed"):
        module.collect_evidence(
            project_root=root,
            output_root=tmp_path / "release",
            timestamp="20260618T120002Z",
            run_command=fake_run,
        )


def test_release_evidence_collector_ironbank_guard_ignores_fixture_words(tmp_path):
    module = _load_module()
    root = tmp_path / "repo"
    root.mkdir()
    (root / "benchmarks").mkdir()
    (root / "assets").mkdir()
    (root / "assets" / "manifest.json").write_text("{}")
    (root / "CHANGELOG.md").write_text("## [Unreleased]\n")
    ironbank = root / "tests" / "ironbank"
    ironbank.mkdir(parents=True)
    (ironbank / "test_words.py").write_text(
        "def test_fixture_words_are_not_disabled_tests():\n"
        "    payload = {'mode': 'rewrite', 'fallback': True}\n"
        "    marker_name = 'slow_sleep'\n"
        "    assert payload['fallback'] is True\n"
        "    assert marker_name.endswith('sleep')\n"
    )

    def fake_run(args, cwd):
        del cwd
        command = " ".join(args)
        if args[:2] == ["git", "status"]:
            return module.CommandResult(command=command, returncode=0, stdout="", stderr="")
        if args[:2] == ["git", "rev-parse"] and args[-1] == "HEAD":
            return module.CommandResult(command=command, returncode=0, stdout="abc123\n", stderr="")
        if args[:2] == ["git", "branch"]:
            return module.CommandResult(
                command=command, returncode=0, stdout="release/test\n", stderr=""
            )
        return module.CommandResult(command=command, returncode=1, stdout="", stderr="unexpected")

    bundle = module.collect_evidence(
        project_root=root,
        output_root=tmp_path / "release",
        timestamp="20260618T120003Z",
        run_command=fake_run,
    )

    manifest = json.loads((bundle / "manifest.json").read_text())
    assert manifest["ironbank_guard"] == {
        "disabled_test_findings": [],
        "files_scanned": 1,
    }


def test_release_evidence_collector_rejects_installed_keychain_helpers(tmp_path):
    module = _load_module()
    home = tmp_path / "home"
    bin_dir = home / ".capsem" / "bin"
    bin_dir.mkdir(parents=True)
    (bin_dir / "capsem-service").write_bytes(b"clean service")
    (bin_dir / "capsem-mcp-builtin").write_bytes(b"stale helper still opens org.capsem.credentials")

    with pytest.raises(RuntimeError, match="Installed Capsem credential store guard failed"):
        module._installed_credential_store_guard(home)


def test_release_evidence_collector_records_clean_installed_binary_guard(tmp_path):
    module = _load_module()
    root = tmp_path / "repo"
    root.mkdir()
    (root / "benchmarks").mkdir()
    (root / "assets").mkdir()
    (root / "assets" / "manifest.json").write_text("{}")
    (root / "CHANGELOG.md").write_text("## [Unreleased]\n")
    home = tmp_path / "home"
    bin_dir = home / ".capsem" / "bin"
    bin_dir.mkdir(parents=True)
    for name in ["capsem-service", "capsem-mcp-builtin", "capsem-mcp-aggregator"]:
        (bin_dir / name).write_bytes(f"clean {name}".encode())

    def fake_run(args, cwd):
        del cwd
        command = " ".join(args)
        if args[:2] == ["git", "status"]:
            return module.CommandResult(command=command, returncode=0, stdout="", stderr="")
        if args[:2] == ["git", "rev-parse"] and args[-1] == "HEAD":
            return module.CommandResult(command=command, returncode=0, stdout="abc123\n", stderr="")
        if args[:2] == ["git", "branch"]:
            return module.CommandResult(
                command=command, returncode=0, stdout="release/test\n", stderr=""
            )
        return module.CommandResult(command=command, returncode=1, stdout="", stderr="unexpected")

    bundle = module.collect_evidence(
        project_root=root,
        output_root=tmp_path / "release",
        timestamp="20260618T120004Z",
        run_command=fake_run,
        home=home,
    )

    manifest = json.loads((bundle / "manifest.json").read_text())
    assert manifest["installed_credential_store_guard"] == {
        "installed_bin_dir": str(bin_dir),
        "files_scanned": 3,
        "forbidden_findings": [],
        "present": True,
    }


def test_release_evidence_collector_cli_default_output(tmp_path, monkeypatch):
    module = _load_module()
    root = tmp_path / "repo"
    root.mkdir()
    (root / "benchmarks").mkdir()

    def fake_collect(*, project_root, output_root, timestamp):
        del project_root, timestamp
        output_root.mkdir(parents=True, exist_ok=True)
        (output_root / "manifest.json").write_text("{}")
        return output_root

    monkeypatch.setattr(module, "collect_evidence", fake_collect)

    rc = module.main(
        [
            "--project-root",
            str(root),
            "--output",
            str(tmp_path / "bundle"),
            "--timestamp",
            "20260618T120001Z",
        ]
    )

    assert rc == 0
    assert (tmp_path / "bundle" / "manifest.json").exists()
