"""Built distributions must work without the checkout or build dependencies."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

import pytest


@pytest.mark.parametrize("kind", ["wheel", "sdist"])
def test_built_distribution_installs_and_runs_in_an_isolated_runtime(kind: str) -> None:
    root = Path(__file__).resolve().parents[3]
    config = tomllib.loads((root / "config/gate.toml").read_text())["sdk_python"]
    manifest = tomllib.loads((root / config["manifest"]).read_text())["project"]
    output = root / config["build_output"]
    stem = f"{manifest['name'].replace('-', '_')}-{manifest['version']}"
    archives = list(output.glob(f"{stem}-*.whl" if kind == "wheel" else f"{stem}.tar.gz"))
    assert len(archives) == 1, (
        f"expected one current {kind} in {output}; run the SDK package build before this test"
    )
    archive = archives[0].resolve()
    assert archive.is_file() and not archives[0].is_symlink()
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    probe = root / config["tests"] / "image_package_acceptance.py"

    # The outer bounded test owner supplies its external scratch root. The
    # probe refuses any workspace imports, editable install or build packages.
    with tempfile.TemporaryDirectory(prefix=f"capsem-sdk-{kind}-") as temporary:
        work = Path(temporary)
        assert not work.resolve().is_relative_to(root)
        prefix = work / "runtime"
        python = prefix / "bin/python"
        commands = [
            ["uv", "venv", "--offline", "--python", sys.executable, str(prefix)],
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--python",
                str(python),
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
        assert Path(report["prefix"]).resolve() == prefix.resolve()
        assert report["http_paths"] == [
            "/images?refresh=true",
            "/images/pull",
            "/images/pull",
        ]
        assert report["lifecycle_paths"] == [
            "/vms/restore-vm/start",
            "/vms/restore-vm/resume",
            "/vms/restore-vm/start",
            "/vms/restore-vm/info",
            "/vms/restore-vm/resume",
            "/vms/restore-vm/info",
            "/vms/slow/info",
        ]
    assert not work.exists(), "the installed runtime must be removed after acceptance"
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
