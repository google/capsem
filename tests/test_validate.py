"""Tests for capsem.builder.validate -- compiler-style config linter."""

from __future__ import annotations

import textwrap
from pathlib import Path

import pytest

from capsem.builder.validate import (
    Severity,
    find_toml_line,
    validate_guest,
)

PROJECT_ROOT = Path(__file__).parent.parent

MINIMAL_BUILD_TOML = """\
[build]
compression = "zstd"
compression_level = 15

[build.architectures.arm64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
node_major = 24
"""

CAPSEM_MCP_TOML = """\
[capsem]
name = "Capsem"
description = "Built-in Capsem MCP server"
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
enabled = true
"""

WEB_SECURITY_TOML = """\
[web]

[web.search.google]
name = "Google"
enabled = true
domains = ["www.google.com"]
allow_get = true

[web.registry.pypi]
name = "PyPI"
enabled = true
domains = ["pypi.org"]
allow_get = true

[web.repository.github]
name = "GitHub"
enabled = true
domains = ["github.com"]
allow_get = true
allow_post = true
"""

VM_RESOURCES_TOML = """\
[resources]
cpu_count = 4
ram_gb = 4
scratch_disk_size_gb = 16
"""

VM_ENVIRONMENT_TOML = """\
[environment.shell]
term = "xterm-256color"
home = "/root"
path = "/usr/bin:/bin"
lang = "C"

[environment.tls]
ca_bundle = "/etc/ssl/certs/ca-certificates.crt"
"""

PYTHON_PACKAGES_TOML = """\
[python]
name = "Python Packages"
manager = "uv"
install_cmd = "uv pip install --system"
packages = ["pytest", "requests"]

[python.network]
name = "PyPI"
domains = ["pypi.org"]
allow_get = true
"""


@pytest.fixture
def guest_valid(tmp_path: Path) -> Path:
    config = tmp_path / "guest" / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)

    mcp = config / "mcp"
    mcp.mkdir()
    (mcp / "capsem.toml").write_text(CAPSEM_MCP_TOML)

    sec = config / "security"
    sec.mkdir()
    (sec / "web.toml").write_text(WEB_SECURITY_TOML)

    vm = config / "vm"
    vm.mkdir()
    (vm / "resources.toml").write_text(VM_RESOURCES_TOML)
    (vm / "environment.toml").write_text(VM_ENVIRONMENT_TOML)

    pkg = config / "packages"
    pkg.mkdir()
    (pkg / "python.toml").write_text(PYTHON_PACKAGES_TOML)

    kernel = config / "kernel"
    kernel.mkdir()
    (kernel / "defconfig.arm64").write_text("# test defconfig\n")

    return tmp_path / "guest"


def _codes(diags) -> set[str]:
    return {d.code for d in diags}


def _errors(diags):
    return [d for d in diags if d.severity is Severity.ERROR]


def test_find_toml_line_section_and_key() -> None:
    text = "[web.search.google]\nname = \"Google\"\nallow_get = true\n"
    assert find_toml_line(text, "web.search.google") == 1
    assert find_toml_line(text, "allow_get") == 3
    assert find_toml_line(text, "missing") is None


def test_valid_config_has_no_errors(guest_valid: Path) -> None:
    assert _errors(validate_guest(guest_valid)) == []


def test_missing_config_directory_is_e001(tmp_path: Path) -> None:
    diags = validate_guest(tmp_path / "guest")
    assert _codes(diags) == {"E001"}


def test_missing_build_toml_is_e001(tmp_path: Path) -> None:
    (tmp_path / "guest" / "config").mkdir(parents=True)
    diags = validate_guest(tmp_path / "guest")
    assert _codes(diags) == {"E001"}


def test_invalid_toml_is_e002(guest_valid: Path) -> None:
    (guest_valid / "config" / "mcp" / "capsem.toml").write_text("[broken")
    assert "E002" in _codes(validate_guest(guest_valid))


def test_pydantic_validation_error_is_e003(guest_valid: Path) -> None:
    (guest_valid / "config" / "build.toml").write_text("[build]\ncompression = 'zstd'\n")
    assert "E003" in _codes(validate_guest(guest_valid))


def test_empty_package_list_is_e004(guest_valid: Path) -> None:
    (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
        [python]
        name = "Python"
        manager = "uv"
        install_cmd = "uv pip install"
        packages = []
    """))
    assert "E004" in _codes(validate_guest(guest_valid))


def test_invalid_package_manager_is_e005(guest_valid: Path) -> None:
    (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
        [python]
        name = "Python"
        manager = "conda"
        install_cmd = "conda install"
        packages = ["numpy"]
    """))
    assert "E005" in _codes(validate_guest(guest_valid))


@pytest.mark.parametrize("domain", ["https://example.com", "example.com/path", "example.com:443", "   "])
def test_invalid_web_domain_is_e006(guest_valid: Path, domain: str) -> None:
    (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent(f"""\
        [web]

        [web.search.bad]
        name = "Bad"
        enabled = true
        domains = ["{domain}"]
        allow_get = true
    """))
    assert "E006" in _codes(validate_guest(guest_valid))


def test_duplicate_mcp_and_package_keys_are_e008(guest_valid: Path) -> None:
    (guest_valid / "config" / "mcp" / "capsem2.toml").write_text(CAPSEM_MCP_TOML)
    (guest_valid / "config" / "packages" / "python2.toml").write_text(PYTHON_PACKAGES_TOML)
    codes = _codes(validate_guest(guest_valid))
    assert "E008" in codes


def test_missing_kernel_defconfig_is_e300(guest_valid: Path) -> None:
    (guest_valid / "config" / "kernel" / "defconfig.arm64").unlink()
    assert "E300" in _codes(validate_guest(guest_valid))


def test_artifact_validation_checks_required_files(guest_valid: Path, tmp_path: Path) -> None:
    artifacts = tmp_path / "artifacts"
    artifacts.mkdir()
    diags = validate_guest(guest_valid, artifacts_dir=artifacts)
    codes = _codes(diags)
    assert "E301" in codes
    assert "E302" in codes


def test_missing_registry_for_package_set_is_w001(guest_valid: Path) -> None:
    (guest_valid / "config" / "security" / "web.toml").write_text("[web]\n")
    assert "W001" in _codes(validate_guest(guest_valid))


def test_dev_package_warning_is_w002(guest_valid: Path) -> None:
    (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
        [python]
        name = "Python"
        manager = "uv"
        install_cmd = "uv pip install"
        packages = ["openssl-dev"]
    """))
    assert "W002" in _codes(validate_guest(guest_valid))


def test_secret_in_mcp_or_shell_is_w003(guest_valid: Path) -> None:
    (guest_valid / "config" / "mcp" / "capsem.toml").write_text(textwrap.dedent("""\
        [capsem]
        name = "Capsem"
        transport = "stdio"
        command = "/run/capsem-mcp-server"
        headers = { Authorization = "Bearer ghp_realtoken12345678901234567890" }
    """))
    assert "W003" in _codes(validate_guest(guest_valid))


def test_package_set_without_network_is_w004(guest_valid: Path) -> None:
    (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
        [python]
        name = "Python"
        manager = "uv"
        install_cmd = "uv pip install"
        packages = ["pytest"]
    """))
    assert "W004" in _codes(validate_guest(guest_valid))


def test_broad_web_wildcard_is_w007(guest_valid: Path) -> None:
    (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
        [web]

        [web.search.anything]
        name = "Anything"
        enabled = true
        domains = ["*.com"]
        allow_get = true
    """))
    assert "W007" in _codes(validate_guest(guest_valid))


def test_shell_metacharacter_in_install_cmd_is_w009(guest_valid: Path) -> None:
    (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
        [python]
        name = "Python"
        manager = "uv"
        install_cmd = "uv pip install; rm -rf /"
        packages = ["pytest"]
    """))
    assert "W009" in _codes(validate_guest(guest_valid))


def test_bad_path_is_w010(guest_valid: Path) -> None:
    (guest_valid / "config" / "vm" / "environment.toml").write_text(textwrap.dedent("""\
        [environment.shell]
        term = "xterm-256color"
        home = "/root"
        path = "/opt/custom"
        lang = "C"
    """))
    assert "W010" in _codes(validate_guest(guest_valid))


def test_unknown_rust_target_is_w012(guest_valid: Path) -> None:
    build = MINIMAL_BUILD_TOML.replace(
        'rust_target = "aarch64-unknown-linux-musl"',
        'rust_target = "aarch64-unknown-linux-gnu"',
    )
    (guest_valid / "config" / "build.toml").write_text(build)
    assert "W012" in _codes(validate_guest(guest_valid))


def test_real_guest_config_has_no_validation_errors() -> None:
    errors = _errors(validate_guest(PROJECT_ROOT / "guest"))
    assert errors == []
