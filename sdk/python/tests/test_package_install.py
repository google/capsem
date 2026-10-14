"""Built distributions must work without the checkout or build dependencies."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from pathlib import Path

import pytest


def _run(command: list[str], *, cwd: Path) -> None:
    result = subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _build_sdist_wheel(archive: Path, work: Path) -> Path:
    source = work / "sdist-source"
    source.mkdir()
    with tarfile.open(archive, "r:gz") as bundle:
        bundle.extractall(source, filter="data")
    roots = list(source.iterdir())
    assert len(roots) == 1 and roots[0].is_dir() and not roots[0].is_symlink()

    output = work / "sdist-wheel"
    _run(
        [
            sys.executable,
            "-m",
            "build",
            "--wheel",
            "--no-isolation",
            "--outdir",
            str(output),
            str(roots[0]),
        ],
        cwd=work,
    )
    wheels = list(output.glob("*.whl"))
    assert len(wheels) == 1
    return wheels[0]


@pytest.mark.parametrize("kind", ["wheel", "sdist"])
def test_built_distribution_installs_and_runs_in_an_isolated_runtime(kind: str) -> None:
    root = Path(__file__).resolve().parents[3]
    config = tomllib.loads((root / "config/gate.toml").read_text())["sdk_python"]
    manifest_path = root / config["manifest"]
    manifest = tomllib.loads(manifest_path.read_text())["project"]
    lockfiles = list((root / config["project"]).glob("*.lock"))
    assert len(lockfiles) == 1
    lockfile = lockfiles[0]
    selected = os.environ.get("CAPSEM_SDK_PACKAGE_ARCHIVE_DIR")
    output = Path(selected) if selected else root / config["build_output"]
    assert output.is_absolute()
    stem = f"{manifest['name'].replace('-', '_')}-{manifest['version']}"
    archives = list(
        output.glob(f"{stem}-*.whl" if kind == "wheel" else f"{stem}.tar.gz")
    )
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
        runtime_project = work / "runtime-project"
        runtime_project.mkdir()
        shutil.copyfile(manifest_path, runtime_project / manifest_path.name)
        shutil.copyfile(lockfile, runtime_project / lockfile.name)
        prefix = runtime_project / ".venv"
        python = prefix / "bin/python"
        installable = _build_sdist_wheel(archive, work) if kind == "sdist" else archive
        _run(
            [
                "uv",
                "sync",
                "--offline",
                "--project",
                str(runtime_project),
                "--frozen",
                "--no-dev",
                "--no-install-project",
            ],
            cwd=work,
        )
        _run(
            [
                "uv",
                "pip",
                "install",
                "--offline",
                "--no-deps",
                "--python",
                str(python),
                str(installable),
            ],
            cwd=work,
        )
        _run(["uv", "pip", "check", "--python", str(python)], cwd=work)

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
        assert report["credential_paths"] == [
            "/credentials/inject",
            "/credentials/inject",
        ]
        assert report["private_input_redacted"] is True
        assert report["python_version"] == list(sys.version_info[:3])
        assert Path(report["executable"]).resolve() == python.resolve()
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
        if receipt_dir := os.environ.get("CAPSEM_SDK_PACKAGE_RECEIPT_DIR"):
            destination = Path(receipt_dir)
            assert destination.is_absolute() and destination.is_dir()
            shutil.copyfile(report_path, destination / f"python-{kind}-receipt.json")
    assert not work.exists(), "the installed runtime must be removed after acceptance"
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
