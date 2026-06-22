"""Dev runtime version checks and execution tests."""

import json
import textwrap
import zipfile

import pytest

from conftest import run


def _write_python_wheel(output_dir, distribution, module, module_source):
    """Create a tiny pure-Python wheel without touching a package index."""
    version = "0.1.0"
    wheel_name = f"{distribution.replace('-', '_')}-{version}-py3-none-any.whl"
    wheel_path = output_dir / wheel_name
    dist_info = f"{distribution.replace('-', '_')}-{version}.dist-info"
    files = {
        f"{module}/__init__.py": textwrap.dedent(module_source).lstrip(),
        f"{dist_info}/METADATA": (
            "Metadata-Version: 2.1\n"
            f"Name: {distribution}\n"
            f"Version: {version}\n"
        ),
        f"{dist_info}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: capsem-doctor\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n"
        ),
    }
    record_rows = [f"{path},," for path in files]
    record_rows.append(f"{dist_info}/RECORD,,")
    files[f"{dist_info}/RECORD"] = "\n".join(record_rows) + "\n"
    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for path, data in files.items():
            zf.writestr(path, data)
    return wheel_path


def _write_npm_package(output_dir, name):
    package_dir = output_dir / name
    bin_dir = package_dir / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    (package_dir / "package.json").write_text(
        json.dumps(
            {
                "name": name,
                "version": "0.1.0",
                "main": "index.js",
                "bin": {name: "bin/cli.js"},
            }
        )
    )
    (package_dir / "index.js").write_text(
        "exports.capitalize = (value) => value.charAt(0).toUpperCase() + value.slice(1);\n"
    )
    cli = bin_dir / "cli.js"
    cli.write_text("#!/usr/bin/env node\nconsole.log('capsem-npm-ok')\n")
    cli.chmod(0o755)
    return package_dir


def _write_deb_package(output_dir):
    root = output_dir / "capsem-apt-hello"
    debian = root / "DEBIAN"
    bin_dir = root / "usr/local/bin"
    debian.mkdir(parents=True, exist_ok=True)
    bin_dir.mkdir(parents=True, exist_ok=True)
    (debian / "control").write_text(
        textwrap.dedent(
            """\
            Package: capsem-apt-hello
            Version: 0.1.0
            Section: utils
            Priority: optional
            Architecture: all
            Maintainer: Capsem Doctor <doctor@capsem.local>
            Description: Hermetic local package-manager probe
            """
        )
    )
    binary = bin_dir / "capsem-apt-hello"
    binary.write_text("#!/bin/sh\necho capsem-apt-ok\n")
    binary.chmod(0o755)
    deb_path = output_dir / "capsem-apt-hello.deb"
    result = run(f"dpkg-deb --build {root} {deb_path}", timeout=15)
    assert result.returncode == 0, f"dpkg-deb --build failed: {result.stdout} {result.stderr}"
    return deb_path


@pytest.mark.parametrize("runtime", ["python3", "node", "npm", "pip3", "uv", "git"])
def test_runtime_version(runtime):
    """Each dev runtime must respond to --version."""
    result = run(f"{runtime} --version")
    assert result.returncode == 0, f"{runtime} --version failed: {result.stderr}"


def test_pip_install_works(output_dir):
    """pip install must work without PEP 668 or permission errors.

    The guest VM activates a venv at /root/.venv so packages install
    to a writable location (rootfs is read-only).
    """
    wheel = _write_python_wheel(
        output_dir,
        "capsem-pip-hello",
        "capsem_pip_hello",
        """
        __version__ = "0.1.0"
        def ping():
            return "capsem-pip-ok"
        """,
    )
    result = run(f"pip install --no-index {wheel} 2>&1", timeout=30)
    assert result.returncode == 0, f"pip install failed: {result.stdout}"
    assert "externally-managed" not in result.stdout.lower(), (
        "PEP 668 EXTERNALLY-MANAGED error not suppressed"
    )
    result = run("python3 -c 'import capsem_pip_hello; print(capsem_pip_hello.ping())'")
    assert result.returncode == 0, f"import local pip wheel failed: {result.stderr}"
    assert "capsem-pip-ok" in result.stdout


def test_uv_pip_install_works(output_dir):
    """uv pip install must work inside the activated venv."""
    wheel = _write_python_wheel(
        output_dir,
        "capsem-uv-wheel",
        "capsem_uv_wheel",
        """
        def marker():
            return "capsem-uv-wheel-ok"
        """,
    )
    result = run(
        "uv pip install --python /root/.venv/bin/python "
        f"--no-index --find-links {wheel.parent} capsem-uv-wheel==0.1.0 2>&1",
        timeout=30,
    )
    assert result.returncode == 0, f"uv pip install failed: {result.stdout}"
    result = run("/root/.venv/bin/python -c 'import capsem_uv_wheel; print(capsem_uv_wheel.marker())'")
    assert result.returncode == 0, f"import local uv wheel failed: {result.stderr}"
    assert "capsem-uv-wheel-ok" in result.stdout


def test_uv_add_package_works(output_dir):
    """uv pip install a real package and verify it imports."""
    wheel = _write_python_wheel(
        output_dir,
        "capsem-uv-extra",
        "capsem_uv_extra",
        """
        def naturalsize(value):
            return f"{value} bytes"
        """,
    )
    result = run(
        f"uv pip install --python /root/.venv/bin/python --no-index {wheel} 2>&1",
        timeout=30,
    )
    assert result.returncode == 0, f"uv pip install local wheel failed: {result.stdout}"
    result = run(
        "/root/.venv/bin/python -c 'import capsem_uv_extra; "
        "print(capsem_uv_extra.naturalsize(1024))'"
    )
    assert result.returncode == 0, f"import local uv package failed: {result.stderr}"
    assert "1024 bytes" in result.stdout


def test_npm_install_global_works(output_dir):
    """npm install -g must work (prefix set to /opt/ai-clis, writable via overlayfs)."""
    package = _write_npm_package(output_dir, "capsem-npm-global")
    result = run(f"npm install -g file:{package} 2>&1", timeout=30)
    assert result.returncode == 0, f"npm install -g failed: {result.stdout}"
    result = run("capsem-npm-global 2>&1")
    assert result.returncode == 0, f"local npm bin not found after npm install -g: {result.stderr}"
    assert "capsem-npm-ok" in result.stdout


def test_apt_install_works(output_dir):
    """apt-get install must work (overlayfs upper is writable)."""
    deb = _write_deb_package(output_dir)
    result = run(f"apt-get install -y -qq {deb} 2>&1", timeout=60)
    assert result.returncode == 0, f"apt-get install local deb failed: {result.stdout}"
    result = run("capsem-apt-hello")
    assert result.returncode == 0, f"local deb binary not found after apt install: {result.stderr}"
    assert "capsem-apt-ok" in result.stdout


def test_remote_apt_https_install_works():
    """Runtime apt must fetch over HTTPS, install, and run the package."""
    update = run(
        "apt-get "
        "-o Acquire::Check-Valid-Until=false "
        "-o Acquire::Check-Date=false "
        "update 2>&1",
        timeout=180,
    )
    assert update.returncode == 0, f"remote apt-get update failed:\n{update.stdout}"
    assert "https://deb.debian.org" in update.stdout, (
        f"runtime apt sources did not use HTTPS debian.org:\n{update.stdout}"
    )
    assert "Certificate verification failed" not in update.stdout
    assert "No system certificates available" not in update.stdout

    install = run(
        "DEBIAN_FRONTEND=noninteractive "
        "apt-get install -y -qq --no-install-recommends hello 2>&1",
        timeout=180,
    )
    assert install.returncode == 0, f"remote apt install failed:\n{install.stdout}"
    hello = run("hello", timeout=15)
    assert hello.returncode == 0, f"remote apt package binary failed:\n{hello.stdout}\n{hello.stderr}"
    assert "Hello, world!" in hello.stdout


def test_tmux_works():
    """tmux must launch and list sessions."""
    result = run("tmux new-session -d -s test-session 2>&1 && tmux list-sessions 2>&1 && tmux kill-session -t test-session 2>&1", timeout=10)
    assert result.returncode == 0, f"tmux failed: {result.stdout} {result.stderr}"
    assert "test-session" in result.stdout


def test_npm_install_local_works(output_dir):
    """npm install (local) must work in a writable directory."""
    project = output_dir / "npm_test"
    package = _write_npm_package(output_dir, "capsem-npm-local")
    cmds = " && ".join([
        f"mkdir -p {project}",
        f"cd {project}",
        "npm init -y",
        f"npm install file:{package}",
        "node -e 'const pkg = require(\"capsem-npm-local\"); console.log(pkg.capitalize(\"works\"))'",
    ])
    result = run(cmds, timeout=30)
    assert result.returncode == 0, f"npm install failed: {result.stdout}\n{result.stderr}"
    assert "Works" in result.stdout


def test_python_execution(output_dir):
    """Python can import stdlib, write JSON, and read it back."""
    out_file = output_dir / "python_exec_test.json"
    code = f"""
import json, os, math
data = {{"pi": math.pi, "pid": os.getpid(), "ok": True}}
with open("{out_file}", "w") as f:
    json.dump(data, f)
"""
    result = run(f'python3 -c \'{code}\'')
    assert result.returncode == 0, f"python3 failed: {result.stderr}"
    assert out_file.exists(), f"{out_file} not created"
    data = json.loads(out_file.read_text())
    assert data["ok"] is True
    assert abs(data["pi"] - 3.14159265) < 0.001


def test_node_execution(output_dir):
    """Node.js can write a JSON file via fs module."""
    out_file = output_dir / "node_exec_test.json"
    js_code = (
        'const fs = require("fs"); '
        f'fs.writeFileSync("{out_file}", '
        'JSON.stringify({node: true, v: process.version}));'
    )
    result = run(f"node -e '{js_code}'")
    assert result.returncode == 0, f"node failed: {result.stderr}"
    assert out_file.exists(), f"{out_file} not created"
    data = json.loads(out_file.read_text())
    assert data["node"] is True


def test_zstd_roundtrip_works(output_dir):
    """zstd must compress and decompress bytes without changing content."""
    payload = output_dir / "zstd_payload.txt"
    compressed = output_dir / "zstd_payload.txt.zst"
    restored = output_dir / "zstd_payload.roundtrip.txt"
    payload.write_text("capsem-zstd-ok\n" * 64)

    result = run(f"zstd -q -f {payload} -o {compressed}", timeout=15)
    assert result.returncode == 0, f"zstd compress failed: {result.stdout}\n{result.stderr}"
    assert compressed.exists(), f"{compressed} not created"

    result = run(f"zstd -q -d -f {compressed} -o {restored}", timeout=15)
    assert result.returncode == 0, f"zstd decompress failed: {result.stdout}\n{result.stderr}"
    result = run(f"cmp {payload} {restored}")
    assert result.returncode == 0, f"zstd roundtrip changed bytes: {result.stdout}\n{result.stderr}"


def test_git_workflow(output_dir):
    """Git can init, configure, commit, and show log."""
    repo = output_dir / "git_test_repo"
    cmds = " && ".join([
        f"rm -rf {repo}",
        f"mkdir -p {repo}",
        f"cd {repo}",
        "git init",
        "git config user.email test@capsem.local",
        "git config user.name capsem-test",
        "echo hello > readme.txt",
        "git add readme.txt",
        'git commit -m "test commit"',
        "git log --oneline",
    ])
    result = run(cmds, timeout=15)
    assert result.returncode == 0, f"git workflow failed: {result.stderr}"
    assert "test commit" in result.stdout
