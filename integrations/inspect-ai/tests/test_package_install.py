"""Built inspect-capsem-sandbox distributions must work without the checkout or build dependencies."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

import pytest


def _ensure_built_archive(root: Path, section: dict[str, str], pattern: str) -> Path:
    output = root / section["build_output"]
    archives = list(output.glob(pattern))
    pkg_dir = root / (section.get("package_dir") or section["source"])
    latest_src = max((p.stat().st_mtime for p in pkg_dir.rglob("*.py")), default=0.0)
    if not archives or archives[0].stat().st_mtime < latest_src:
        build_res = subprocess.run(
            [
                sys.executable,
                "-m",
                "build",
                "--no-isolation",
                "--outdir",
                str(output),
                str(root / section["project"]),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert build_res.returncode == 0, build_res.stdout + build_res.stderr
        archives = list(output.glob(pattern))
    assert len(archives) == 1, f"expected one current archive matching {pattern} in {output}"
    archive = archives[0].resolve()
    assert archive.is_file() and not archives[0].is_symlink()
    return archive


@pytest.mark.parametrize("kind", ["wheel", "sdist"])
def test_built_distribution_installs_and_runs_in_an_isolated_runtime(kind: str) -> None:
    root = Path(__file__).resolve().parents[3]
    gate_cfg = tomllib.loads((root / "config/gate.toml").read_text())
    sdk_cfg = gate_cfg["sdk_python"]
    sdk_manifest = tomllib.loads((root / sdk_cfg["manifest"]).read_text())["project"]
    sdk_stem = f"{sdk_manifest['name'].replace('-', '_')}-{sdk_manifest['version']}"
    sdk_wheel = _ensure_built_archive(root, sdk_cfg, f"{sdk_stem}-*.whl")

    config = gate_cfg["integrations_inspect_ai"]
    project_python = root / config["project"] / ".venv/bin/python"
    base_python = str(project_python.resolve()) if project_python.exists() else sys.executable
    manifest = tomllib.loads((root / config["manifest"]).read_text())["project"]
    stem = f"{manifest['name'].replace('-', '_')}-{manifest['version']}"
    pattern = f"{stem}-*.whl" if kind == "wheel" else f"{stem}.tar.gz"
    archive = _ensure_built_archive(root, config, pattern)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    probe = root / config["tests"] / "image_package_acceptance.py"

    with tempfile.TemporaryDirectory(prefix=f"inspect-capsem-{kind}-") as temporary:
        work = Path(temporary)
        assert not work.resolve().is_relative_to(root)
        prefix = work / "runtime"
        python = prefix / "bin/python"
        commands = [
            [
                "env",
                f"UV_PROJECT_ENVIRONMENT={prefix}",
                "uv",
                "sync",
                "--project",
                str(root / config["project"]),
                "--frozen",
                "--no-dev",
                "--no-install-project",
                "--no-install-local",
                "--offline",
                "--python",
                base_python,
            ],
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--no-deps",
                "--python",
                str(python),
                str(sdk_wheel),
                str(archive),
            ],
        ]
        for command in commands:
            result = subprocess.run(
                command,
                cwd=work,
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
            assert result.returncode == 0, result.stdout + result.stderr

        report_path = work / "acceptance.json"
        result = subprocess.run(
            [
                str(python),
                "-I",
                str(probe),
                "--archive",
                str(archive),
                "--source-root",
                str(root),
                "--output",
                str(report_path),
            ],
            cwd=work,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in result.stdout
        report = json.loads(report_path.read_text())
        assert report["ok"] and report["isolated"]
        assert report["sha256"] == digest
        assert report["version"] == manifest["version"]
        assert report["entry_point"] == "inspect_capsem._registry"
        assert Path(report["prefix"]).resolve() == prefix.resolve()
    assert not work.exists(), "the installed runtime must be removed after acceptance"
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
