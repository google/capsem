"""Runtime checks for the minimal guest: Python, its venv, and apt.

Applications and their toolchains (node, uv, git, AI CLIs) come from OCI
images, which tests/images/ proves; the runtime rootfs does not ship them.
"""

import json
import textwrap
import zipfile

import pytest

from .diagnostic_support import run


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


@pytest.mark.parametrize("runtime", ["python3", "pip3"])
def test_runtime_version(runtime):
    """Each runtime the minimal guest ships must respond to --version."""
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


def test_apt_install_works(output_dir):
    """apt-get install must work (overlayfs upper is writable)."""
    deb = _write_deb_package(output_dir)
    result = run(f"apt-get install -y -qq {deb} 2>&1", timeout=60)
    assert result.returncode == 0, f"apt-get install local deb failed: {result.stdout}"
    result = run("capsem-apt-hello")
    assert result.returncode == 0, f"local deb binary not found after apt install: {result.stderr}"
    assert "capsem-apt-ok" in result.stdout


def test_apt_https_trust_is_readable_by_sandbox_user():
    """Apt must retain HTTPS sources and `_apt` access to the real CA bundle."""
    sources = run(
        "(grep -Rhs '^URIs: https://deb.debian.org' "
        "/etc/apt/sources.list.d 2>/dev/null || true; "
        "grep -hs '^deb .*https://deb.debian.org' "
        "/etc/apt/sources.list 2>/dev/null || true) | "
        "grep -F 'https://deb.debian.org'"
    )
    assert sources.returncode == 0, (
        f"runtime apt sources are not HTTPS debian.org sources:\n{sources.stdout}"
    )
    assert "https://deb.debian.org" in sources.stdout

    trust = run(
        "runuser -u _apt -- test -r /etc/ssl/certs/ca-certificates.crt",
        timeout=15,
    )
    assert trust.returncode == 0, (
        "apt sandbox user cannot read the system TLS trust bundle; "
        f"stdout={trust.stdout!r} stderr={trust.stderr!r}"
    )


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
