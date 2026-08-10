from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path

import pytest

from tests.helpers import service

PROJECT_ROOT = Path(__file__).resolve().parents[1]
RUNNER_PATH = PROJECT_ROOT / "scripts" / "run-installed-winterfell.py"
REQUIRED_BINARIES = (
    "capsem-service",
    "capsem-process",
    "capsem-gateway",
    "capsem-mcp",
)


def _load_runner():
    spec = importlib.util.spec_from_file_location("run_installed_winterfell", RUNNER_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _installed_roots(tmp_path: Path) -> tuple[Path, Path, Path]:
    bin_dir = tmp_path / "installed" / "bin"
    assets_dir = tmp_path / "installed" / "assets"
    profiles_dir = tmp_path / "installed" / "profiles"
    bin_dir.mkdir(parents=True)
    assets_dir.mkdir(parents=True)
    profile_dir = profiles_dir / "code"
    profile_dir.mkdir(parents=True)
    (assets_dir / "manifest.json").write_text('{"profiles":{"code":{}}}\n')
    (profile_dir / "profile.toml").write_text('id = "code"\n')
    for name in REQUIRED_BINARIES:
        binary = bin_dir / name
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(0o755)
    return bin_dir, assets_dir, profiles_dir


def _environment(
    bin_dir: Path,
    assets_dir: Path,
    profiles_dir: Path,
) -> dict[str, str]:
    return {
        "CAPSEM_WINTERFELL_BIN_DIR": str(bin_dir),
        "CAPSEM_WINTERFELL_ASSETS_DIR": str(assets_dir),
        "CAPSEM_WINTERFELL_PROFILES_DIR": str(profiles_dir),
    }


def test_default_winterfell_roots_preserve_the_development_suite() -> None:
    roots = service.resolve_winterfell_artifact_roots({})

    assert roots.installed is False
    assert roots.binary_dir == PROJECT_ROOT / "target" / "debug"
    assert roots.profiles_dir == PROJECT_ROOT / "target" / "config" / "profiles"
    assert roots.assets_dir.parent == PROJECT_ROOT / "assets"


def test_development_winterfell_honours_the_functional_content_selector(
    tmp_path: Path,
) -> None:
    assets = tmp_path / "verified-assets"
    profiles = tmp_path / "verified-profiles"
    roots = service.resolve_winterfell_artifact_roots(
        {
            "CAPSEM_ASSETS_DIR": str(assets),
            "CAPSEM_PROFILES_DIR": str(profiles),
        }
    )

    architecture = "arm64" if os.uname().machine == "arm64" else "x86_64"
    assert roots.installed is False
    assert roots.assets_dir == assets / architecture
    assert roots.profiles_dir == profiles


@pytest.mark.parametrize(
    "present",
    [
        ("CAPSEM_WINTERFELL_BIN_DIR",),
        ("CAPSEM_WINTERFELL_ASSETS_DIR",),
        ("CAPSEM_WINTERFELL_PROFILES_DIR",),
        ("CAPSEM_WINTERFELL_BIN_DIR", "CAPSEM_WINTERFELL_ASSETS_DIR"),
    ],
)
def test_installed_winterfell_override_is_all_or_nothing(
    present: tuple[str, ...],
    tmp_path: Path,
) -> None:
    environment = {name: str(tmp_path / name) for name in present}

    with pytest.raises(RuntimeError, match="all three installed artifact roots"):
        service.resolve_winterfell_artifact_roots(environment)


def test_installed_winterfell_roots_accept_one_complete_installed_cohort(
    tmp_path: Path,
) -> None:
    bin_dir, assets_dir, profiles_dir = _installed_roots(tmp_path)

    roots = service.resolve_winterfell_artifact_roots(
        _environment(bin_dir, assets_dir, profiles_dir)
    )

    assert roots.installed is True
    assert roots.binary("capsem-mcp") == bin_dir / "capsem-mcp"
    assert roots.assets_dir == assets_dir
    assert roots.profiles_dir == profiles_dir


@pytest.mark.parametrize(
    ("bin_dir", "assets_dir", "profiles_dir"),
    [
        (
            PROJECT_ROOT / "target" / "debug",
            PROJECT_ROOT / "assets",
            PROJECT_ROOT / "target" / "config" / "profiles",
        ),
        (
            PROJECT_ROOT / "target" / "debug",
            PROJECT_ROOT / "assets" / "arm64",
            PROJECT_ROOT / "target" / "config" / "profiles",
        ),
    ],
)
def test_installed_winterfell_rejects_source_built_roots(
    bin_dir: Path,
    assets_dir: Path,
    profiles_dir: Path,
) -> None:
    with pytest.raises(RuntimeError, match="source-built"):
        service.resolve_winterfell_artifact_roots(_environment(bin_dir, assets_dir, profiles_dir))


def test_installed_winterfell_rejects_binary_symlinks_into_target_debug(
    tmp_path: Path,
) -> None:
    bin_dir, assets_dir, profiles_dir = _installed_roots(tmp_path)
    source_binary = PROJECT_ROOT / "target" / "debug" / "capsem-mcp"
    if not source_binary.is_file():
        pytest.skip("source MCP binary has not been built")
    (bin_dir / "capsem-mcp").unlink()
    (bin_dir / "capsem-mcp").symlink_to(source_binary)

    with pytest.raises(RuntimeError, match="source-built"):
        service.resolve_winterfell_artifact_roots(_environment(bin_dir, assets_dir, profiles_dir))


def test_runner_executes_only_winterfell_against_exact_installed_roots(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _load_runner()
    bin_dir, assets_dir, profiles_dir = _installed_roots(tmp_path)
    evidence = tmp_path / "winterfell.json"
    captured: dict[str, object] = {}

    class Result:
        returncode = 0

    def fake_run(command, **kwargs):
        captured["command"] = command
        captured.update(kwargs)
        return Result()

    monkeypatch.setattr(module.subprocess, "run", fake_run)

    result = module.main(
        [
            "--bin-dir",
            str(bin_dir),
            "--assets-dir",
            str(assets_dir),
            "--profiles-dir",
            str(profiles_dir),
            "--evidence-out",
            str(evidence),
        ]
    )

    assert result == 0
    assert captured["command"] == [
        os.fspath(Path(module.sys.executable)),
        "-m",
        "pytest",
        "tests/capsem-mcp/test_winterfell_rw.py",
        "tests/capsem-mcp/test_winterfell_exec.py",
        "-q",
    ]
    child_environment = captured["env"]
    assert child_environment["CAPSEM_WINTERFELL_BIN_DIR"] == str(bin_dir)
    assert child_environment["CAPSEM_WINTERFELL_ASSETS_DIR"] == str(assets_dir)
    assert child_environment["CAPSEM_WINTERFELL_PROFILES_DIR"] == str(profiles_dir)
    assert captured["cwd"] == PROJECT_ROOT
    report = json.loads(evidence.read_text())
    assert report == {
        "schema": "capsem.installed_winterfell.v1",
        "passed": True,
        "roots": {
            "assets": str(assets_dir),
            "binaries": str(bin_dir),
            "profiles": str(profiles_dir),
        },
    }


def test_runner_records_failure_and_returns_pytest_status(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    module = _load_runner()
    bin_dir, assets_dir, profiles_dir = _installed_roots(tmp_path)
    evidence = tmp_path / "winterfell.json"

    class Result:
        returncode = 7

    monkeypatch.setattr(module.subprocess, "run", lambda *args, **kwargs: Result())

    result = module.main(
        [
            "--bin-dir",
            str(bin_dir),
            "--assets-dir",
            str(assets_dir),
            "--profiles-dir",
            str(profiles_dir),
            "--evidence-out",
            str(evidence),
        ]
    )

    assert result == 7
    assert json.loads(evidence.read_text())["passed"] is False
