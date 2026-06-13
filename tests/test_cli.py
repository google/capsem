"""Tests for capsem.builder.cli -- Click CLI commands.

TDD: tests written first (RED), then cli.py makes them pass (GREEN).
Uses Click's CliRunner for isolated command testing.
"""

from __future__ import annotations

import json
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
        assert "package_sets" in data
        assert "ai_providers" not in data

    def test_inspect_minimal(self, tmp_path):
        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "arm64" in result.output


class TestRemovedAuthoringCommands:
    """Profile/admin materialization owns authoring; builder scaffolds are gone."""

    @pytest.mark.parametrize("command", ["init", "new", "add"])
    def test_scaffold_commands_are_removed(self, command):
        runner = CliRunner()
        result = runner.invoke(cli, [command])
        assert result.exit_code != 0
        assert "No such command" in result.output


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

    def test_add_to_nonexistent_guest_is_not_a_command(self, tmp_path):
        runner = CliRunner()
        result = runner.invoke(cli, ["add", "packages", "test", "--dir", str(tmp_path / "nope")])
        assert result.exit_code != 0
        assert "No such command" in result.output


# ---------------------------------------------------------------------------
# doctor command
# ---------------------------------------------------------------------------


class TestDoctorCommand:
    """Tests for the doctor command."""

    def test_doctor_runs_profile_contract(self):
        """Doctor command runs and produces output."""
        from unittest.mock import patch

        from capsem.builder.doctor import CheckResult

        runner = CliRunner()
        with patch("capsem.builder.doctor.run_all_checks") as mock:
            mock.return_value = [
                CheckResult(name="profile-contract", passed=True, detail="profile code")
            ]
            result = runner.invoke(cli, ["doctor", "--profile", "code", "--config-root", "config"])

        assert result.exit_code == 0
        assert "capsem-builder doctor" in result.output
        assert "passed" in result.output
        mock.assert_called_once()
        assert mock.call_args.kwargs["profile_id"] == "code"

    def test_doctor_rejects_positional_guest_dir(self):
        """Doctor must not accept a positional guest config directory."""
        runner = CliRunner()
        result = runner.invoke(cli, ["doctor", "guest/"])
        assert result.exit_code != 0
        assert "unexpected extra argument" in result.output.lower()


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
        """Without docker, build should show fix guidance."""
        from unittest.mock import patch

        from capsem.builder.doctor import CheckResult

        guest = _write_minimal_guest(tmp_path)
        runner = CliRunner()
        with patch("capsem.builder.docker.check_container_runtime") as mock:
            mock.return_value = CheckResult(
                name="container-runtime", passed=False,
                detail="docker not found", fix="brew install colima docker",
            )
            result = runner.invoke(cli, ["build", str(guest)])
        assert result.exit_code != 0
        assert "container-runtime" in result.output or "docker" in result.output


# ---------------------------------------------------------------------------
# Corporate image test
# ---------------------------------------------------------------------------


class TestCorporateImage:
    """Prove that a customized guest config produces a different image."""

    def _write_corp_config(self, guest_dir: Path) -> None:
        """Create a corporate image config with custom packages."""
        config = guest_dir / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text(MINIMAL_BUILD_TOML)

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
        (pkg_dir / "npm.toml").write_text("""\
[npm]
name = "Node CLIs"
manager = "npm"
install_cmd = "npm install -g"
packages = ["@corp/internal-agent-cli"]
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

    def test_inspect_shows_custom_packages(self, tmp_path):
        """Inspect shows corporate package sets."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["inspect", str(guest)])
        assert result.exit_code == 0
        assert "apt" in result.output
        assert "npm" in result.output

    def test_dry_run_has_custom_npm_package(self, tmp_path):
        """Rendered Dockerfile contains the corporate npm package."""
        guest = tmp_path / "corp"
        self._write_corp_config(guest)
        runner = CliRunner()
        result = runner.invoke(cli, ["build", str(guest), "--dry-run"])
        assert result.exit_code == 0
        assert "@corp/internal-agent-cli" in result.output

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

    def test_no_default_provider_installs(self, tmp_path):
        """Corporate config does not install dead default provider packages."""
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
        # Dead default provider packages should not be in the npm install block.
        assert "@openai/codex" not in npm_block
        # Claude curl installer should not be present either
        assert "claude.ai/install.sh" not in dockerfile
        # But custom package-set CLIs should be.
        assert "@corp/internal-agent-cli" in npm_block

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
        assert "@corp/internal-agent-cli" in corp_df
        assert "@corp/internal-agent-cli" not in default_df
