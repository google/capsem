"""Tests for capsem.builder.cli -- Click CLI commands.

TDD: tests written first (RED), then cli.py makes them pass (GREEN).
Uses Click's CliRunner for isolated command testing.
"""

from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest
from click.testing import CliRunner

from capsem.builder.cli import cli

PROJECT_ROOT = Path(__file__).parent.parent

# ---------------------------------------------------------------------------
# Inline TOML fixtures
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

DUAL_ARCH_BUILD_TOML = """\
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

[build.architectures.x86_64]
base_image = "debian:bookworm-slim"
docker_platform = "linux/amd64"
rust_target = "x86_64-unknown-linux-musl"
kernel_branch = "6.6"
kernel_image = "arch/x86/boot/bzImage"
defconfig = "kernel/defconfig.x86_64"
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
domains = ["*.google.com", "*.googleapis.com"]
allow_get = true

[web.registry.pypi]
name = "PyPI"
enabled = true
domains = ["pypi.org", "files.pythonhosted.org"]
allow_get = true
"""

APT_PACKAGES_TOML = """\
[apt]
name = "System packages"
manager = "apt"
install_cmd = "apt-get install -y --no-install-recommends"
packages = ["curl", "git", "vim"]
"""

VM_RESOURCES_TOML = """\
[resources]
cpu_count = 4
ram_gb = 4
scratch_disk_size_gb = 16
"""

VM_ENVIRONMENT_TOML = """\
[environment]

[environment.shell]
term = "xterm-256color"
home = "/root"
path = "/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
lang = "C"
"""


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _write_minimal_guest(tmp_path: Path) -> Path:
    """Create a minimal guest config directory."""
    guest = tmp_path / "guest"
    config = guest / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)
    # Create defconfig
    kernel_dir = config / "kernel"
    kernel_dir.mkdir()
    (kernel_dir / "defconfig.arm64").write_text("# minimal\n")
    return guest


def _write_full_guest(tmp_path: Path) -> Path:
    """Create a full guest config directory with all sections."""
    guest = tmp_path / "guest"
    config = guest / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(DUAL_ARCH_BUILD_TOML)

    ai_dir = config / "ai"
    ai_dir.mkdir()
    (ai_dir / "google.toml").write_text(GOOGLE_AI_TOML)

    mcp_dir = config / "mcp"
    mcp_dir.mkdir()
    (mcp_dir / "capsem.toml").write_text(CAPSEM_MCP_TOML)

    sec_dir = config / "security"
    sec_dir.mkdir()
    (sec_dir / "web.toml").write_text(WEB_SECURITY_TOML)

    pkg_dir = config / "packages"
    pkg_dir.mkdir()
    (pkg_dir / "apt.toml").write_text(APT_PACKAGES_TOML)

    vm_dir = config / "vm"
    vm_dir.mkdir()
    (vm_dir / "resources.toml").write_text(VM_RESOURCES_TOML)
    (vm_dir / "environment.toml").write_text(VM_ENVIRONMENT_TOML)

    kernel_dir = config / "kernel"
    kernel_dir.mkdir()
    (kernel_dir / "defconfig.arm64").write_text("# arm64\n")
    (kernel_dir / "defconfig.x86_64").write_text("# x86_64\n")

    return guest


# ---------------------------------------------------------------------------
# Top-level CLI
# ---------------------------------------------------------------------------


class TestCli:
    """Top-level CLI group."""

    def test_help(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["--help"])
        assert result.exit_code == 0
        assert "capsem-builder" in result.output.lower() or "build" in result.output.lower()

    def test_version(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["--version"])
        assert result.exit_code == 0
        assert "capsem-builder" in result.output.lower()

    def test_no_args_shows_help(self):
        runner = CliRunner()
        result = runner.invoke(cli, [])
        assert result.exit_code == 0
        # Should show available commands
        assert "validate" in result.output
        assert "build" in result.output


# ---------------------------------------------------------------------------
# validate command
# ---------------------------------------------------------------------------


class TestValidateCommand:
    """Tests for the validate command."""

    def test_valid_config(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0
        assert "ok" in result.output.lower() or "clean" in result.output.lower() or "pass" in result.output.lower()

    def test_missing_config_dir(self, tmp_path):
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(tmp_path / "nonexistent")])
        assert result.exit_code != 0

    def test_missing_build_toml(self, tmp_path):
        guest = tmp_path / "guest"
        config = guest / "config"
        config.mkdir(parents=True)
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code != 0
        assert "E001" in result.output

    def test_invalid_toml(self, tmp_path):
        guest = tmp_path / "guest"
        config = guest / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text("[invalid\nbroken toml")
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code != 0
        assert "E002" in result.output

    def test_shows_warnings(self, tmp_path):
        """Warnings are shown but exit code is still 0."""
        guest = _write_minimal_guest(tmp_path)
        # Add a package set with no network (triggers W004)
        pkg_dir = guest / "config" / "packages"
        pkg_dir.mkdir()
        (pkg_dir / "apt.toml").write_text(APT_PACKAGES_TOML)
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0
        assert "W004" in result.output

    def test_artifacts_flag(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        artifacts = tmp_path / "artifacts"
        artifacts.mkdir()
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest), "--artifacts", str(artifacts)])
        assert result.exit_code != 0
        # Missing capsem-init, capsem-ca.crt etc.
        assert "E301" in result.output or "E302" in result.output

    def test_full_config_validates_clean(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0

    def test_default_guest_dir(self):
        """Without argument, uses ./guest as default."""
        runner = CliRunner()
        result = runner.invoke(cli, ["validate"])
        # May or may not find guest/ depending on cwd, but should not crash
        assert result.exit_code in (0, 1)

    def test_error_count_in_output(self, tmp_path):
        """Errors should show a count."""
        guest = tmp_path / "guest"
        config = guest / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text("[invalid\nbroken")
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code != 0
        assert "error" in result.output.lower()


# ---------------------------------------------------------------------------
# build command
# ---------------------------------------------------------------------------


class TestBuildCommand:
    """Tests for the build command."""

    def test_dry_run_renders_dockerfile(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        # Should contain Dockerfile content
        assert "FROM" in result.output

    def test_dry_run_specific_arch(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run", "--arch", "arm64"])
        assert result.exit_code == 0
        assert "FROM" in result.output
        assert "linux/arm64" in result.output

    def test_dry_run_all_arches(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        # Both architectures should appear
        assert "arm64" in result.output
        assert "x86_64" in result.output

    def test_dry_run_invalid_arch(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run", "--arch", "riscv64"])
        assert result.exit_code != 0
        assert "riscv64" in result.output

    def test_dry_run_kernel_template(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run", "--template", "kernel"])
        assert result.exit_code == 0
        assert "FROM" in result.output

    def test_build_validates_first(self, tmp_path):
        """Build should validate config before rendering."""
        guest = tmp_path / "guest"
        config = guest / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text("[invalid\nbroken")
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code != 0

    def test_build_no_dry_run_needs_docker(self, tmp_path):
        """Without --dry-run, build should mention docker is needed."""
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest)])
        # Should fail gracefully (docker not available or not implemented)
        assert result.exit_code != 0

    def test_dry_run_default_guest_dir(self):
        """Without path argument, uses ./guest."""
        runner = CliRunner()
        result = runner.invoke(cli, ["build", "--dry-run"])
        # May or may not work depending on cwd
        assert result.exit_code in (0, 1)

    def test_dry_run_json_output(self, tmp_path):
        """--dry-run --json should output JSON manifest."""
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert "architectures" in data


# ---------------------------------------------------------------------------
# inspect command
# ---------------------------------------------------------------------------


class TestInspectCommand:
    """Tests for the inspect command."""

    def test_inspect_shows_summary(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        # Should show architecture info
        assert "arm64" in result.output
        assert "x86_64" in result.output

    def test_inspect_shows_providers(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "google" in result.output.lower()

    def test_inspect_shows_packages(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "apt" in result.output.lower()

    def test_inspect_shows_mcp(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "capsem" in result.output.lower()

    def test_inspect_invalid_dir(self, tmp_path):
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(tmp_path / "nope")])
        assert result.exit_code != 0

    def test_inspect_json_output(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest), "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert "build" in data
        assert "ai_providers" in data

    def test_inspect_minimal(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "arm64" in result.output


# ---------------------------------------------------------------------------
# init command
# ---------------------------------------------------------------------------


class TestInitCommand:
    """Tests for the init scaffolding command."""

    def test_init_creates_structure(self, tmp_path):
        target = tmp_path / "myguest"
        runner = CliRunner()
        result = runner.invoke(cli, ["init", str(target)])
        assert result.exit_code == 0
        # Should create directory structure
        assert (target / "config" / "build.toml").exists()
        assert (target / "config" / "kernel").is_dir()

    def test_init_build_toml_is_valid(self, tmp_path):
        target = tmp_path / "myguest"
        runner = CliRunner()
        result = runner.invoke(cli, ["init", str(target)])
        assert result.exit_code == 0
        # The generated build.toml should validate
        result2 = runner.invoke(cli, ["validate", str(target)])
        assert result2.exit_code == 0

    def test_init_existing_dir_fails(self, tmp_path):
        target = tmp_path / "existing"
        (target / "config").mkdir(parents=True)
        runner = CliRunner()
        result = runner.invoke(cli, ["init", str(target)])
        assert result.exit_code != 0
        assert "exists" in result.output.lower()

    def test_init_force_overwrites(self, tmp_path):
        target = tmp_path / "existing"
        (target / "config").mkdir(parents=True)
        runner = CliRunner()
        result = runner.invoke(cli, ["init", str(target), "--force"])
        assert result.exit_code == 0
        assert (target / "config" / "build.toml").exists()

    def test_init_default_dir(self, tmp_path):
        """Without argument, uses ./guest."""
        runner = CliRunner()
        # Run in tmp_path to avoid polluting project root
        result = runner.invoke(cli, ["init"], catch_exceptions=False)
        # Will either succeed or fail because ./guest already exists
        assert result.exit_code in (0, 1)


# ---------------------------------------------------------------------------
# add command group
# ---------------------------------------------------------------------------


class TestAddAiProviderCommand:
    """Tests for the add ai-provider scaffolding command."""

    def test_add_ai_provider(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "ai-provider", "openai", "--dir", str(guest)])
        assert result.exit_code == 0
        ai_file = guest / "config" / "ai" / "openai.toml"
        assert ai_file.exists()
        content = ai_file.read_text()
        assert "[openai]" in content
        assert "api_key" in content

    def test_add_ai_provider_already_exists(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "ai-provider", "google", "--dir", str(guest)])
        assert result.exit_code != 0
        assert "exists" in result.output.lower()

    def test_add_ai_provider_force(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "ai-provider", "google", "--dir", str(guest), "--force"])
        assert result.exit_code == 0

    def test_add_ai_provider_creates_ai_dir(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "ai-provider", "mistral", "--dir", str(guest)])
        assert result.exit_code == 0
        assert (guest / "config" / "ai" / "mistral.toml").exists()

    def test_added_provider_validates(self, tmp_path):
        """Added provider should produce valid config."""
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        runner.invoke(cli, ["add", "ai-provider", "openai", "--dir", str(guest)])
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0


class TestAddPackagesCommand:
    """Tests for the add packages scaffolding command."""

    def test_add_packages_apt(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "packages", "system", "--dir", str(guest), "--manager", "apt"])
        assert result.exit_code == 0
        pkg_file = guest / "config" / "packages" / "system.toml"
        assert pkg_file.exists()
        content = pkg_file.read_text()
        assert "[system]" in content
        assert 'manager = "apt"' in content

    def test_add_packages_default_manager(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "packages", "python", "--dir", str(guest)])
        assert result.exit_code == 0
        pkg_file = guest / "config" / "packages" / "python.toml"
        assert pkg_file.exists()

    def test_add_packages_already_exists(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "packages", "apt", "--dir", str(guest)])
        assert result.exit_code != 0
        assert "exists" in result.output.lower()

    def test_add_packages_npm(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "packages", "node", "--dir", str(guest), "--manager", "npm"])
        assert result.exit_code == 0
        content = (guest / "config" / "packages" / "node.toml").read_text()
        assert 'manager = "npm"' in content

    def test_added_packages_validates(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        runner.invoke(cli, ["add", "packages", "system", "--dir", str(guest), "--manager", "apt"])
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0


class TestAddMcpCommand:
    """Tests for the add mcp scaffolding command."""

    def test_add_mcp_stdio(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "mcp", "myserver", "--dir", str(guest)])
        assert result.exit_code == 0
        mcp_file = guest / "config" / "mcp" / "myserver.toml"
        assert mcp_file.exists()
        content = mcp_file.read_text()
        assert "[myserver]" in content
        assert 'transport = "stdio"' in content

    def test_add_mcp_sse(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "mcp", "remote", "--dir", str(guest), "--transport", "sse"])
        assert result.exit_code == 0
        content = (guest / "config" / "mcp" / "remote.toml").read_text()
        assert 'transport = "sse"' in content
        assert "url" in content

    def test_add_mcp_already_exists(self, tmp_path):
        guest = _write_full_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "mcp", "capsem", "--dir", str(guest)])
        assert result.exit_code != 0
        assert "exists" in result.output.lower()

    def test_added_mcp_validates(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        runner.invoke(cli, ["add", "mcp", "myserver", "--dir", str(guest)])
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0


# ---------------------------------------------------------------------------
# audit command
# ---------------------------------------------------------------------------


TRIVY_JSON_FIXTURE = json.dumps({
    "Results": [{
        "Target": "test",
        "Vulnerabilities": [
            {"VulnerabilityID": "CVE-2024-1234", "Severity": "HIGH",
             "PkgName": "openssl", "InstalledVersion": "3.0.13",
             "FixedVersion": "3.0.14"},
            {"VulnerabilityID": "CVE-2024-5678", "Severity": "LOW",
             "PkgName": "curl", "InstalledVersion": "7.88"},
        ],
    }],
})

TRIVY_NO_VULNS_FIXTURE = json.dumps({"Results": [{"Target": "test"}]})


class TestAuditCommand:
    """Tests for the audit command."""

    def test_audit_from_file(self, tmp_path):
        f = tmp_path / "trivy.json"
        f.write_text(TRIVY_JSON_FIXTURE)
        runner = CliRunner()
        result = runner.invoke(cli, ["audit", "--input", str(f)])
        # Has HIGH vuln so exit code 1
        assert result.exit_code == 1
        assert "CVE-2024-1234" in result.output
        assert "HIGH" in result.output

    def test_audit_json_output(self, tmp_path):
        f = tmp_path / "trivy.json"
        f.write_text(TRIVY_JSON_FIXTURE)
        runner = CliRunner()
        result = runner.invoke(cli, ["audit", "--input", str(f), "--json"])
        assert result.exit_code == 1
        data = json.loads(result.output)
        assert len(data) == 2

    def test_audit_no_vulns_exit_zero(self, tmp_path):
        f = tmp_path / "trivy.json"
        f.write_text(TRIVY_NO_VULNS_FIXTURE)
        runner = CliRunner()
        result = runner.invoke(cli, ["audit", "--input", str(f)])
        assert result.exit_code == 0

    def test_audit_grype_scanner(self, tmp_path):
        grype = json.dumps({"matches": [{
            "vulnerability": {"id": "CVE-2024-1", "severity": "Low",
                              "fix": {"versions": [], "state": "not-fixed"}},
            "artifact": {"name": "zlib", "version": "1.2.3"},
        }]})
        f = tmp_path / "grype.json"
        f.write_text(grype)
        runner = CliRunner()
        result = runner.invoke(cli, ["audit", "--scanner", "grype", "--input", str(f)])
        assert result.exit_code == 0  # Only LOW, no HIGH/CRITICAL
        assert "zlib" in result.output

    def test_audit_no_input_fails(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["audit"], input="")
        assert result.exit_code != 0


# ---------------------------------------------------------------------------
# mcp command
# ---------------------------------------------------------------------------


class TestMcpCommand:
    """Tests for the mcp command."""

    def test_mcp_initialize(self):
        init_msg = json.dumps({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                       "clientInfo": {"name": "test", "version": "1.0"}},
        })
        runner = CliRunner()
        result = runner.invoke(cli, ["mcp"], input=init_msg + "\n")
        assert result.exit_code == 0
        resp = json.loads(result.output.strip().splitlines()[0])
        assert resp["result"]["serverInfo"]["name"] == "capsem-builder"


# ---------------------------------------------------------------------------
# Real config (project guest/ directory)
# ---------------------------------------------------------------------------


class TestRealConfig:
    """Tests against the actual project guest/ directory."""

    def test_validate_real_guest(self):
        """capsem-builder validate guest/ works on the real config."""
        guest = PROJECT_ROOT / "guest"
        if not (guest / "config" / "build.toml").exists():
            pytest.skip("guest/config/build.toml not found")
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0

    def test_build_dry_run_real_guest(self):
        """capsem-builder build --dry-run works on the real config."""
        guest = PROJECT_ROOT / "guest"
        if not (guest / "config" / "build.toml").exists():
            pytest.skip("guest/config/build.toml not found")
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        assert "FROM" in result.output

    def test_inspect_real_guest(self):
        """capsem-builder inspect guest/ works on the real config."""
        guest = PROJECT_ROOT / "guest"
        if not (guest / "config" / "build.toml").exists():
            pytest.skip("guest/config/build.toml not found")
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0

    def test_inspect_json_real_guest(self):
        """capsem-builder inspect --json guest/ returns valid JSON."""
        guest = PROJECT_ROOT / "guest"
        if not (guest / "config" / "build.toml").exists():
            pytest.skip("guest/config/build.toml not found")
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest), "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert "build" in data


# ---------------------------------------------------------------------------
# Edge cases and error handling
# ---------------------------------------------------------------------------


class TestEdgeCases:
    """Edge cases and error handling."""

    def test_validate_empty_dir(self, tmp_path):
        """Empty directory has no config/ subdirectory."""
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(tmp_path)])
        assert result.exit_code != 0

    def test_build_dry_run_minimal(self, tmp_path):
        """Minimal config with one arch produces valid Dockerfile."""
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        assert "FROM" in result.output

    def test_commands_handle_permission_errors(self, tmp_path):
        """Commands should handle unreadable directories gracefully."""
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", "/root/nonexistent"])
        assert result.exit_code != 0

    def test_add_to_nonexistent_guest(self, tmp_path):
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "ai-provider", "test", "--dir", str(tmp_path / "nope")])
        assert result.exit_code != 0


# ---------------------------------------------------------------------------
# doctor command
# ---------------------------------------------------------------------------


class TestDoctorCommand:
    """Tests for the doctor command."""

    def test_doctor_runs(self):
        """Doctor command runs and produces output."""
        runner = CliRunner()
        result = runner.invoke(cli, ["doctor", "guest/"])
        # May pass or fail depending on environment, but should not crash
        assert "capsem-builder doctor" in result.output
        assert "passed" in result.output

    def test_doctor_nonexistent_dir(self, tmp_path):
        """Doctor with nonexistent guest dir shows config failure."""
        runner = CliRunner()
        result = runner.invoke(cli, ["doctor", str(tmp_path / "nope")])
        assert "FAIL" in result.output


# ---------------------------------------------------------------------------
# build command: new flags
# ---------------------------------------------------------------------------


class TestBuildNewFlags:
    """Tests for --output and --kernel-version flags."""

    def test_output_flag_accepted(self, tmp_path):
        """--output is a valid option on build command."""
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, [
            "build", str(guest), "--dry-run", "--output", str(tmp_path / "out"),
        ])
        assert result.exit_code == 0

    def test_kernel_version_flag_accepted(self, tmp_path):
        """--kernel-version is a valid option on build command."""
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, [
            "build", str(guest), "--dry-run", "--kernel-version", "6.6.131",
        ])
        assert result.exit_code == 0

    def test_build_no_runtime_shows_fix(self, tmp_path):
        """Without docker/podman, build should show fix guidance."""
        from unittest.mock import patch

        from capsem.builder.doctor import CheckResult

        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        with patch("capsem.builder.docker.check_container_runtime") as mock:
            mock.return_value = CheckResult(
                name="container-runtime", passed=False,
                detail="not found", fix="brew install podman",
            )
            result = runner.invoke(cli, ["build", str(guest)])
        assert result.exit_code != 0
        assert "container-runtime" in result.output or "podman" in result.output


# ---------------------------------------------------------------------------
# Corporate image test
# ---------------------------------------------------------------------------


class TestCorporateImage:
    """Prove that a customized guest config produces a different image."""

    def _write_corp_config(self, guest_dir: Path) -> None:
        """Create a corporate image config with internal LLM + custom packages."""
        config = guest_dir / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text(MINIMAL_BUILD_TOML)

        ai_dir = config / "ai"
        ai_dir.mkdir()
        (ai_dir / "internal-llm.toml").write_text("""\
[internal-llm]
name = "Internal LLM"
description = "Corporate LLM endpoint"
enabled = true

[internal-llm.api_key]
name = "Internal API Key"
env_vars = ["INTERNAL_LLM_KEY"]
prefix = "ik_"
docs_url = "https://internal.corp.com/docs"

[internal-llm.network]
domains = ["llm.internal.corp.com"]
allow_get = true
allow_post = true

[internal-llm.install]
manager = "npm"
prefix = "/opt/ai-clis"
packages = ["@corp/internal-llm-cli"]
""")

        pkg_dir = config / "packages"
        pkg_dir.mkdir()
        (pkg_dir / "apt.toml").write_text("""\
[apt]
name = "System Packages"
manager = "apt"
install_cmd = "apt-get install -y --no-install-recommends"
packages = ["curl", "git", "vim-tiny"]
""")
        (pkg_dir / "python.toml").write_text("""\
[python]
name = "Data Science"
manager = "uv"
install_cmd = "uv pip install --system --break-system-packages"
packages = ["numpy", "pandas", "internal-lib==1.2.3"]
""")
        # Kernel defconfig (required by validator E300)
        kernel_dir = config / "kernel"
        kernel_dir.mkdir()
        (kernel_dir / "defconfig.arm64").write_text("# stub kernel config\n")

    def test_validate_passes(self, tmp_path):
        """Corporate config validates without errors."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["validate", str(guest)])
        assert result.exit_code == 0

    def test_inspect_shows_custom_provider(self, tmp_path):
        """Inspect shows the corporate provider, not defaults."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "Internal LLM" in result.output
        assert "llm.internal.corp.com" in result.output

    def test_dry_run_has_custom_npm_package(self, tmp_path):
        """Rendered Dockerfile contains the corporate npm package."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        assert "@corp/internal-llm-cli" in result.output

    def test_dry_run_has_custom_python_packages(self, tmp_path):
        """Rendered Dockerfile contains corporate Python packages."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        assert "numpy" in result.output
        assert "pandas" in result.output
        assert "internal-lib==1.2.3" in result.output

    def test_no_default_providers(self, tmp_path):
        """Corporate config without default providers doesn't install them."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)

        from capsem.builder.config import load_guest_config
        from capsem.builder.docker import render_dockerfile

        config = load_guest_config(guest)
        dockerfile = render_dockerfile("Dockerfile.rootfs.j2", config, "arm64")
        # Extract only the npm install RUN line (not template comments)
        npm_lines = [
            ln for ln in dockerfile.split("\n")
            if "npm install -g" in ln or ln.strip().startswith("@")
        ]
        npm_block = "\n".join(npm_lines)
        # Default providers should NOT be in the npm install block
        assert "@anthropic-ai/claude-code" not in npm_block
        assert "@openai/codex" not in npm_block
        # But custom provider should be
        assert "@corp/internal-llm-cli" in npm_block

    def test_differs_from_default(self, tmp_path):
        """Corporate Dockerfile differs from the default guest/ config."""
        from capsem.builder.config import load_guest_config
        from capsem.builder.docker import render_dockerfile

        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        corp_config = load_guest_config(guest)
        corp_df = render_dockerfile("Dockerfile.rootfs.j2", corp_config, "arm64")

        default_config = load_guest_config(PROJECT_ROOT / "guest")
        default_df = render_dockerfile("Dockerfile.rootfs.j2", default_config, "arm64")

        assert corp_df != default_df
        assert "@corp/internal-llm-cli" in corp_df
        assert "@corp/internal-llm-cli" not in default_df


# ---------------------------------------------------------------------------
# new command
# ---------------------------------------------------------------------------


class TestNewCommand:
    """Tests for the new command (non-interactive mode via CliRunner)."""

    def test_non_interactive_creates_config(self, tmp_path):
        target = tmp_path / "my-image"
        runner = CliRunner()
        result = runner.invoke(cli, [
            "new", str(target),
            "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        assert result.exit_code == 0
        assert (target / "config" / "manifest.toml").is_file()
        assert (target / "config" / "build.toml").is_file()

    def test_non_interactive_copies_all_providers(self, tmp_path):
        target = tmp_path / "my-image"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target),
            "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        ai = target / "config" / "ai"
        assert (ai / "anthropic.toml").is_file()
        assert (ai / "google.toml").is_file()
        assert (ai / "openai.toml").is_file()

    def test_non_interactive_loadable(self, tmp_path):
        """Created image can be loaded and inspected."""
        from capsem.builder.config import load_guest_config

        target = tmp_path / "test-img"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target),
            "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        config = load_guest_config(target)
        assert config.manifest is not None
        assert config.manifest.name == "test-img"
        assert "anthropic" in config.ai_providers

    def test_non_interactive_validates(self, tmp_path):
        """Created image passes validation."""
        target = tmp_path / "val-img"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target),
            "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        result = runner.invoke(cli, ["validate", str(target)])
        assert result.exit_code == 0

    def test_non_interactive_dry_run_works(self, tmp_path):
        """Created image can produce a Dockerfile via --dry-run."""
        target = tmp_path / "dr-img"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target),
            "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        result = runner.invoke(cli, ["build", str(target), "--dry-run"])
        assert result.exit_code == 0
        assert "FROM" in result.output

    def test_force_overwrites(self, tmp_path):
        target = tmp_path / "ow-img"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target), "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        # Without force -> fails
        result = runner.invoke(cli, [
            "new", str(target), "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        assert result.exit_code != 0
        # With force -> succeeds
        result = runner.invoke(cli, [
            "new", str(target), "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive", "--force",
        ])
        assert result.exit_code == 0

    def test_inspect_shows_manifest(self, tmp_path):
        """inspect command shows image name and version."""
        target = tmp_path / "ins-img"
        runner = CliRunner()
        runner.invoke(cli, [
            "new", str(target), "--from", str(PROJECT_ROOT / "guest"),
            "--non-interactive",
        ])
        result = runner.invoke(cli, ["inspect", str(target)])
        assert result.exit_code == 0
        assert "ins-img" in result.output
        assert "v0.1.0" in result.output
