"""Clean offline SDK consumers, validated before exercising the real gateway."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from collections.abc import Mapping
from pathlib import Path

from helpers.bounded import bounded


def _run(
    command: list[str], work: Path, environment: Mapping[str, str], *, timeout_seconds: int = 60,
) -> str:
    result = subprocess.run(
        bounded(command, timeout_seconds), cwd=work, env=dict(environment),
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return result.stdout


def _archive(output: Path, pattern: str) -> Path:
    archives = list(output.glob(pattern))
    assert len(archives) == 1, f"build the current SDK package first: {output / pattern}"
    assert archives[0].is_file() and not archives[0].is_symlink()
    return archives[0].resolve()


def python_gateway(
    root: Path, environment: Mapping[str, str], *, probe: str = "gateway_acceptance.py",
    success_marker: str = "BRAAVOS_SDK_ACCEPTANCE_OK", timeout_seconds: int = 60,
) -> str:
    settings = tomllib.loads((root / "config/gate.toml").read_text())["sdk_python"]
    manifest = tomllib.loads((root / settings["manifest"]).read_text())["project"]
    stem = f"{manifest['name'].replace('-', '_')}-{manifest['version']}"
    output = root / settings["build_output"]
    results = []
    for kind, pattern in (("wheel", f"{stem}-*.whl"), ("sdist", f"{stem}.tar.gz")):
        archive = _archive(output, pattern)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory(prefix=f"braavos-{kind}-") as temporary:
            work = Path(temporary)
            assert not work.resolve().is_relative_to(root.resolve())
            prefix = work / "runtime"
            python = prefix / "bin/python"
            _run(["uv", "venv", "--offline", "--python", sys.executable, str(prefix)], work, environment)
            _run(["uv", "pip", "install", "--offline", "--python", str(python), str(archive)],
                 work, environment)
            payload = work / "payload.py"
            gateway = work / "gateway.py"
            for name, destination in (("image_package_acceptance.py", payload),
                                      (probe, gateway)):
                shutil.copyfile(root / settings["tests"] / name, destination)
            report = work / "payload.json"
            result = _run([str(python), "-I", str(payload), "--archive", str(archive),
                           "--source-root", str(root), "--output", str(report)], work, environment)
            assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in result
            receipt = json.loads(report.read_text())
            assert receipt["ok"] and receipt["isolated"]
            assert receipt["sha256"] == digest and receipt["version"] == manifest["version"]
            assert Path(receipt["prefix"]).resolve() == prefix.resolve()
            result += _run([str(python), "-I", str(gateway)], work, environment,
                           timeout_seconds=timeout_seconds)
            assert success_marker in result
            results.append(result)
        assert not work.exists()
        assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    return "".join(results)


def typescript_gateway(
    root: Path, environment: Mapping[str, str], *, probe: str = "gateway-acceptance.mjs",
    success_marker: str = "BRAAVOS_SDK_ACCEPTANCE_OK", timeout_seconds: int = 60,
) -> str:
    settings = tomllib.loads((root / "config/gate.toml").read_text())["sdk_typescript"]
    project = root / settings["project"]
    manifest = json.loads((root / settings["manifest"]).read_text())
    stem = manifest["name"].removeprefix("@").replace("/", "-")
    archive = _archive(root / settings["build_output"], f"{stem}-{manifest['version']}.tgz")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    files = {}
    with tarfile.open(archive) as stream:
        for member in stream.getmembers():
            path = Path(member.name)
            assert path.parts[0] == "package" and ".." not in path.parts
            if member.isdir():
                continue
            assert member.isfile(), f"non-regular package payload: {member.name}"
            content = stream.extractfile(member)
            assert content is not None
            files[str(Path(*path.parts[1:]))] = hashlib.sha256(content.read()).hexdigest()
    with tempfile.TemporaryDirectory(prefix="braavos-npm-") as temporary:
        work = Path(temporary)
        assert not work.resolve().is_relative_to(root.resolve())
        (work / "package.json").write_text(json.dumps({"name": "braavos-consumer", "private": True}))
        receipt = work / "payload.json"
        receipt.write_text(json.dumps({"archiveSha256": digest, "files": files}))
        # Resolve exclusively from the clean consumer. The gate owns cache
        # prewarming; acceptance cannot fetch dependencies or run build hooks.
        clean = {key: value for key, value in environment.items() if key not in {"NODE_PATH", "NODE_OPTIONS"}}
        _run(["npm", "install", "--offline", "--omit=dev", "--ignore-scripts", "--no-audit",
              "--no-fund", str(archive)], work, clean)
        payload = work / "payload.mjs"
        gateway = work / "gateway.mjs"
        shutil.copyfile(project / "tools/image-package-acceptance.mjs", payload)
        shutil.copyfile(project / "tools" / probe, gateway)
        report = work / "acceptance.json"
        result = _run(["node", str(payload), str(root), str(archive), str(report), str(receipt)], work, clean)
        assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in result
        verified = json.loads(report.read_text())
        assert verified["ok"] and verified["sha256"] == digest
        assert verified["version"] == manifest["version"]
        result += _run(["node", str(gateway)], work, clean, timeout_seconds=timeout_seconds)
        assert success_marker in result
    assert not work.exists()
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    return result


def inspect_ai_gateway(
    root: Path, environment: Mapping[str, str], *, probe: str = "live_acceptance.py",
    success_marker: str = "INSPECT_CAPSEM_VM_ACCEPTANCE_OK", timeout_seconds: int = 240,
) -> str:
    gate_cfg = tomllib.loads((root / "config/gate.toml").read_text())
    sdk_settings = gate_cfg["sdk_python"]
    sdk_manifest = tomllib.loads((root / sdk_settings["manifest"]).read_text())["project"]
    sdk_stem = f"{sdk_manifest['name'].replace('-', '_')}-{sdk_manifest['version']}"
    sdk_wheel = _archive(root / sdk_settings["build_output"], f"{sdk_stem}-*.whl")
    settings = gate_cfg["integrations_inspect_ai"]
    project_python = root / settings["project"] / ".venv/bin/python"
    base_python = str(project_python.resolve()) if project_python.exists() else sys.executable
    manifest = tomllib.loads((root / settings["manifest"]).read_text())["project"]
    stem = f"{manifest['name'].replace('-', '_')}-{manifest['version']}"
    output = root / settings["build_output"]
    results = []
    for kind, pattern in (("wheel", f"{stem}-*.whl"), ("sdist", f"{stem}.tar.gz")):
        archive = _archive(output, pattern)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory(prefix=f"braavos-inspect-{kind}-") as temporary:
            work = Path(temporary)
            assert not work.resolve().is_relative_to(root.resolve())
            prefix = work / "runtime"
            python = prefix / "bin/python"
            _run(
                [
                    "env", f"UV_PROJECT_ENVIRONMENT={prefix}",
                    "uv", "sync", "--project", str(root / settings["project"]),
                    "--frozen", "--no-dev", "--no-install-project", "--no-install-local",
                    "--offline", "--python", base_python,
                ],
                work, environment,
            )
            _run(
                ["uv", "pip", "install", "--offline", "--no-deps", "--python", str(python), str(sdk_wheel), str(archive)],
                work, environment,
            )
            payload = work / "payload.py"
            gateway = work / "gateway.py"
            tests_dir = root / settings["tests"]
            for name, destination in (("image_package_acceptance.py", payload), (probe, gateway)):
                shutil.copyfile(tests_dir / name, destination)
            fixture_helper = tests_dir / "oci_workload_fixture.py"
            if fixture_helper.is_file():
                shutil.copyfile(fixture_helper, work / "oci_workload_fixture.py")
            report = work / "payload.json"
            result = _run([str(python), "-I", str(payload), "--archive", str(archive),
                           "--source-root", str(root), "--output", str(report)], work, environment)
            assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in result
            receipt = json.loads(report.read_text())
            assert receipt["ok"] and receipt["isolated"]
            assert receipt["sha256"] == digest and receipt["version"] == manifest["version"]
            assert receipt["entry_point"] == "inspect_capsem._registry"
            assert Path(receipt["prefix"]).resolve() == prefix.resolve()
            result += _run([str(python), "-I", str(gateway)], work, environment,
                           timeout_seconds=timeout_seconds)
            assert success_marker in result
            results.append(result)
        assert not work.exists()
        assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
    return "".join(results)

