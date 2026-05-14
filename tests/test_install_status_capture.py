import json
import os
import platform
import subprocess
import sys
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
CAPTURE_SCRIPT = REPO_ROOT / "scripts" / "capture-install-status.py"


def write_fake_capsem(
    tmp_path: Path,
    status_body: str,
    status_code: int,
    debug_body: str = '{"schema":"capsem.debug.v1","ok":true}',
    debug_code: int = 0,
) -> Path:
    fake = tmp_path / "capsem"
    fake.write_text(
        "\n".join(
            [
                "#!/bin/sh",
                'if [ "$1" = "version" ]; then',
                '  echo "capsem 9.9.9 (test-build)"',
                "  exit 0",
                "fi",
                'if [ "$1" = "status" ] && [ "$2" = "--json" ]; then',
                f"  printf '%s\\n' {json.dumps(status_body)}",
                '  echo "status stderr marker" >&2',
                f"  exit {status_code}",
                "fi",
                'if [ "$1" = "debug" ]; then',
                f"  printf '%s\\n' {json.dumps(debug_body)}",
                '  echo "debug stderr marker" >&2',
                f"  exit {debug_code}",
                "fi",
                'echo "unexpected args: $*" >&2',
                "exit 64",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    fake.chmod(0o755)
    return fake


def run_capture(
    tmp_path: Path, fake_capsem: Path, extra_env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["HOME"] = str(tmp_path / "home")
    env["CAPSEM_HOME"] = str(tmp_path / "capsem-home")
    env["CAPSEM_RUN_DIR"] = str(tmp_path / "capsem-home" / "run")
    if extra_env:
        env.update(extra_env)
    system = platform.system()
    if system == "Darwin":
        unit = tmp_path / "home" / "Library" / "LaunchAgents" / "com.capsem.service.plist"
    elif system == "Linux":
        unit = tmp_path / "home" / ".config" / "systemd" / "user" / "capsem.service"
    else:
        unit = None
    if unit is not None:
        unit.parent.mkdir(parents=True)
        unit.write_text("service unit\n", encoding="utf-8")
    (tmp_path / "capsem-home" / "bin").mkdir(parents=True, exist_ok=True)
    (tmp_path / "capsem-home" / "run").mkdir(parents=True, exist_ok=True)
    (tmp_path / "capsem-home" / "assets").mkdir(parents=True, exist_ok=True)
    (tmp_path / "capsem-home" / "bin" / "capsem").write_text("installed\n", encoding="utf-8")
    (tmp_path / "capsem-home" / "assets" / "manifest.json").write_text("{}", encoding="utf-8")
    (tmp_path / "capsem-home" / "run" / "service.pid").write_text("1234\n", encoding="utf-8")
    (tmp_path / "capsem-home" / "run" / "gateway.port").write_text("19222\n", encoding="utf-8")
    (tmp_path / "capsem-home" / "run" / "gateway.token").write_text("secret\n", encoding="utf-8")
    return subprocess.run(
        [
            sys.executable,
            str(CAPTURE_SCRIPT),
            "--capsem-bin",
            str(fake_capsem),
            "--out-dir",
            str(tmp_path / "bundle"),
        ],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )


def test_capture_install_status_preserves_typed_status_failure(tmp_path):
    status = {
        "schema": "capsem.status.v1",
        "ok": False,
        "state": "blocked",
        "checks": {
            "service_endpoint": {
                "state": "blocked",
                "issue_codes": ["service_unreachable"],
            }
        },
        "issues": [
            {
                "code": "service_unreachable",
                "severity": "error",
                "summary": "service is not reachable",
            }
        ],
    }
    result = run_capture(tmp_path, write_fake_capsem(tmp_path, json.dumps(status), 23))

    assert result.returncode == 23
    assert result.stdout.strip() == str(tmp_path / "bundle")

    bundle = tmp_path / "bundle"
    assert json.loads((bundle / "status.json").read_text(encoding="utf-8")) == status
    assert json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))[
        "status_issue_codes"
    ] == ["service_unreachable"]
    assert json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))[
        "status_checks"
    ]["service_endpoint"]["state"] == "blocked"
    assert (bundle / "status.stdout.txt").read_text(encoding="utf-8").strip() == json.dumps(
        status
    )
    assert (bundle / "status.stderr.txt").read_text(encoding="utf-8").strip() == (
        "status stderr marker"
    )
    assert "capsem 9.9.9" in (bundle / "version.stdout.txt").read_text(encoding="utf-8")
    assert json.loads((bundle / "debug.json").read_text(encoding="utf-8")) == {
        "schema": "capsem.debug.v1",
        "ok": True,
    }
    assert (bundle / "debug.stderr.txt").read_text(encoding="utf-8").strip() == (
        "debug stderr marker"
    )
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["commands"]["debug"]["returncode"] == 0
    tree = json.loads((bundle / "capsem-home-tree.json").read_text(encoding="utf-8"))
    assert {"path": "bin/capsem", "kind": "file", "mode": "0o644", "size": 10} in tree
    run_state = json.loads((bundle / "run-state.json").read_text(encoding="utf-8"))
    entries = {entry["path"]: entry for entry in run_state["entries"]}
    assert entries["service.pid"]["contents"] == "1234"
    assert entries["gateway.port"]["contents"] == "19222"
    assert "contents" not in entries["gateway.token"]
    assert entries["gateway.token"]["contents_redacted"] is True
    layout = json.loads((bundle / "install-layout.json").read_text(encoding="utf-8"))
    assert layout["binaries"]["capsem"]["kind"] == "file"
    assert layout["binaries"]["capsem-service"]["kind"] == "missing"
    assert layout["assets"]["manifest.json"]["kind"] == "file"
    assert layout["assets"]["manifest.json.minisig"]["kind"] == "missing"
    if platform.system() in {"Darwin", "Linux"}:
        assert layout["service_unit"]["contents"] == "service unit"


def test_capture_install_status_keeps_raw_output_when_json_is_invalid(tmp_path):
    result = run_capture(tmp_path, write_fake_capsem(tmp_path, "not json", 2))

    assert result.returncode == 2
    bundle = tmp_path / "bundle"
    assert not (bundle / "status.json").exists()
    assert (bundle / "status.stdout.txt").read_text(encoding="utf-8").strip() == "not json"
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["commands"]["status"]["returncode"] == 2
    assert metadata["status_parse_error"]


def test_capture_install_status_keeps_status_exit_when_debug_fails(tmp_path):
    status = {"schema": "capsem.status.v1", "ok": False, "issues": []}
    fake = write_fake_capsem(
        tmp_path,
        json.dumps(status),
        23,
        debug_body="debug failed",
        debug_code=99,
    )

    result = run_capture(tmp_path, fake)

    assert result.returncode == 23
    bundle = tmp_path / "bundle"
    assert not (bundle / "debug.json").exists()
    assert (bundle / "debug.stdout.txt").read_text(encoding="utf-8").strip() == "debug failed"
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["commands"]["status"]["returncode"] == 23
    assert metadata["commands"]["debug"]["returncode"] == 99
    assert metadata["debug_parse_error"]


def test_capture_install_status_pairs_stale_unit_issue_with_unit_contents(tmp_path):
    status = {
        "schema": "capsem.status.v1",
        "ok": False,
        "issues": [
            {
                "code": "service_unit_stale_path",
                "severity": "error",
                "message": "service unit points at an old runtime",
                "details": {
                    "unit_path": "com.capsem.service.plist",
                    "expected_path": "/Users/test/.capsem/bin/capsem-service",
                },
            }
        ],
    }

    result = run_capture(tmp_path, write_fake_capsem(tmp_path, json.dumps(status), 11))

    assert result.returncode == 11
    bundle = tmp_path / "bundle"
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["status_issue_codes"] == ["service_unit_stale_path"]
    layout = json.loads((bundle / "install-layout.json").read_text(encoding="utf-8"))
    if platform.system() in {"Darwin", "Linux"}:
        assert layout["service_unit"]["contents"] == "service unit"


def test_capture_install_status_pairs_app_bundle_issue_with_bundle_state(tmp_path):
    if platform.system() != "Darwin":
        pytest.skip("macOS app bundle evidence is Darwin-only")

    missing_bundle = tmp_path / "Applications" / "Capsem.app"
    status = {
        "schema": "capsem.status.v1",
        "ok": False,
        "issues": [
            {
                "code": "app_bundle_missing",
                "severity": "error",
                "message": "desktop app bundle missing",
                "details": {"path": str(missing_bundle)},
            }
        ],
    }

    result = run_capture(
        tmp_path,
        write_fake_capsem(tmp_path, json.dumps(status), 11),
        {"CAPSEM_APP_BUNDLE": str(missing_bundle)},
    )

    assert result.returncode == 11
    bundle = tmp_path / "bundle"
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["status_issue_codes"] == ["app_bundle_missing"]
    layout = json.loads((bundle / "install-layout.json").read_text(encoding="utf-8"))
    assert layout["macos_app_bundle"] == {"path": "Capsem.app", "kind": "missing"}


def test_capture_install_status_records_saved_vm_state_without_env_values(tmp_path):
    run_dir = tmp_path / "capsem-home" / "run"
    vm_dir = run_dir / "persistent" / "devvm"
    vm_dir.mkdir(parents=True)
    (vm_dir / "state.json").write_text("{}", encoding="utf-8")
    assets_dir = tmp_path / "capsem-home" / "assets" / "arm64"
    assets_dir.mkdir(parents=True)
    rootfs = assets_dir / "rootfs-old.squashfs"
    rootfs.write_text("rootfs", encoding="utf-8")
    registry = {
        "vms": {
            "devvm": {
                "name": "devvm",
                "ram_mb": 4096,
                "cpus": 4,
                "base_version": "2026.0415.1",
                "created_at": "2026-05-13T00:00:00Z",
                "session_dir": str(vm_dir),
                "suspended": True,
                "defunct": False,
                "checkpoint_path": str(vm_dir / "checkpoint.vz"),
                "base_assets": {
                    "asset_version": "2026.0415.1",
                    "asset_arch": "arm64",
                    "rootfs_hash": "abc123",
                    "rootfs_path": str(rootfs),
                },
                "env": {"TOKEN": "secret-value"},
            }
        }
    }
    (run_dir / "persistent_registry.json").write_text(json.dumps(registry), encoding="utf-8")

    result = run_capture(
        tmp_path,
        write_fake_capsem(
            tmp_path,
            json.dumps({"schema": "capsem.status.v1", "ok": True, "issues": []}),
            0,
        ),
    )

    assert result.returncode == 0
    saved = json.loads((tmp_path / "bundle" / "saved-vm-state.json").read_text(encoding="utf-8"))
    assert saved["vm_count"] == 1
    assert saved["registry_vms"]["devvm"]["base_version"] == "2026.0415.1"
    assert saved["registry_vms"]["devvm"]["suspended"] is True
    assert saved["registry_vms"]["devvm"]["env_keys"] == ["TOKEN"]
    asset_refs = saved["registry_vms"]["devvm"]["asset_references"]
    assert asset_refs["asset_version"] == "2026.0415.1"
    assert asset_refs["asset_arch"] == "arm64"
    assert asset_refs["rootfs_hash"] == "abc123"
    assert asset_refs["files"]["rootfs"]["kind"] == "file"
    assert asset_refs["files"]["rootfs"]["path"] == "rootfs-old.squashfs"
    assert "secret-value" not in json.dumps(saved)
    assert any(entry["path"] == "devvm/state.json" for entry in saved["persistent_tree"])


def test_capture_install_status_records_malformed_saved_vm_registry(tmp_path):
    run_dir = tmp_path / "capsem-home" / "run"
    run_dir.mkdir(parents=True)
    (run_dir / "persistent_registry.json").write_text("{not json", encoding="utf-8")

    result = run_capture(
        tmp_path,
        write_fake_capsem(
            tmp_path,
            json.dumps({"schema": "capsem.status.v1", "ok": True, "issues": []}),
            0,
        ),
    )

    assert result.returncode == 0
    saved = json.loads((tmp_path / "bundle" / "saved-vm-state.json").read_text(encoding="utf-8"))
    assert saved["registry"]["kind"] == "file"
    assert saved["registry_parse_error"]
    assert "registry_vms" not in saved


def test_capture_install_status_records_missing_capsem_binary(tmp_path):
    missing = tmp_path / "missing-capsem"
    bundle = tmp_path / "bundle"

    result = subprocess.run(
        [
            sys.executable,
            str(CAPTURE_SCRIPT),
            "--capsem-bin",
            str(missing),
            "--out-dir",
            str(bundle),
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 127
    assert result.stdout.strip() == str(bundle)
    assert "No such file" in (bundle / "status.stderr.txt").read_text(encoding="utf-8")
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["commands"]["version"]["returncode"] == 127
    assert metadata["commands"]["status"]["returncode"] == 127
    assert metadata["status_parse_error"] == "empty stdout"


def test_capture_install_status_records_status_timeout(tmp_path):
    fake = tmp_path / "capsem"
    fake.write_text(
        "\n".join(
            [
                "#!/bin/sh",
                'if [ "$1" = "version" ]; then echo "capsem 9.9.9"; exit 0; fi',
                'if [ "$1" = "status" ] && [ "$2" = "--json" ]; then sleep 2; fi',
                "exit 0",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    fake.chmod(0o755)

    result = subprocess.run(
        [
            sys.executable,
            str(CAPTURE_SCRIPT),
            "--capsem-bin",
            str(fake),
            "--out-dir",
            str(tmp_path / "bundle"),
            "--timeout",
            "0.1",
        ],
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 124
    bundle = tmp_path / "bundle"
    assert "timed out" in (bundle / "status.stderr.txt").read_text(encoding="utf-8")
    metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
    assert metadata["commands"]["status"]["returncode"] == 124
    assert metadata["commands"]["status"]["timed_out"] is True
