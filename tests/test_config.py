"""Tests for capsem.builder.config -- TOML config directory loader + JSON generator.

TDD: tests written first (RED), then config.py makes them pass (GREEN).
Uses tmp_path fixtures with inline TOML strings (no real guest/config/ yet).
"""

from __future__ import annotations

import json
import tomllib
from pathlib import Path

import pytest
from pydantic import ValidationError

from capsem.builder.config import generate_defaults_json, load_guest_config, parse_toml
from capsem.builder.models import (
    Compression,
    GuestImageConfig,
    PackageManager,
)
from capsem.builder.schema import McpTransport

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

[google.install]
manager = "npm"
prefix = "/opt/ai-clis"
packages = ["@google/gemini-cli"]

[google.files.settings_json]
path = "/root/.gemini/settings.json"
content = '{"key": "value"}'
"""

ANTHROPIC_AI_TOML = """\
[anthropic]
name = "Anthropic"
description = "Claude Code AI agent"
enabled = true

[anthropic.api_key]
name = "Anthropic API Key"
env_vars = ["ANTHROPIC_API_KEY"]
prefix = "sk-ant-"

[anthropic.network]
domains = ["*.anthropic.com", "*.claude.com"]
allow_get = true
allow_post = true
"""

PYTHON_PACKAGES_TOML = """\
[python]
name = "Python Packages"
manager = "uv"
install_cmd = "uv pip install --system --break-system-packages"
packages = ["pytest", "numpy", "requests"]

[python.network]
name = "PyPI"
domains = ["pypi.org", "files.pythonhosted.org"]
allow_get = true
"""

CAPSEM_MCP_TOML = """\
[capsem]
name = "Capsem"
description = "Built-in Capsem MCP server for file and snapshot tools"
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
enabled = true
"""

WEB_SECURITY_TOML = """\
[web]
allow_read = false
allow_write = false
custom_allow = ["elie.net", "*.elie.net"]
custom_block = []

[web.search.google]
name = "Google"
enabled = true
domains = ["www.google.com", "google.com"]
allow_get = true

[web.registry.pypi]
name = "PyPI"
enabled = true
domains = ["pypi.org", "files.pythonhosted.org"]
allow_get = true

[web.repository.github]
name = "GitHub"
enabled = true
domains = ["github.com", "*.github.com"]
allow_get = true
allow_post = true
"""

VM_RESOURCES_TOML = """\
[resources]
cpu_count = 4
ram_gb = 4
scratch_disk_size_gb = 16
log_bodies = false
max_body_capture = 4096
retention_days = 30
max_sessions = 100
max_disk_gb = 100
terminated_retention_days = 365
"""

VM_ENVIRONMENT_TOML = """\
[environment.shell]
term = "xterm-256color"
home = "/root"
path = "/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
lang = "C"

[environment.shell.bashrc]
path = "/root/.bashrc"
content = "PS1='$ '"

[environment.shell.tmux_conf]
path = "/root/.tmux.conf"
content = "set -g mouse on"

[environment.tls]
ca_bundle = "/etc/ssl/certs/ca-certificates.crt"
"""


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


@pytest.fixture
def guest_minimal(tmp_path):
    """Create a minimal guest/ directory with only build.toml."""
    config = tmp_path / "guest" / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)
    return tmp_path / "guest"


@pytest.fixture
def guest_full(tmp_path):
    """Create a full guest/ directory with all config files."""
    config = tmp_path / "guest" / "config"
    config.mkdir(parents=True)
    (config / "build.toml").write_text(MINIMAL_BUILD_TOML)

    ai = config / "ai"
    ai.mkdir()
    (ai / "google.toml").write_text(GOOGLE_AI_TOML)
    (ai / "anthropic.toml").write_text(ANTHROPIC_AI_TOML)

    pkg = config / "packages"
    pkg.mkdir()
    (pkg / "python.toml").write_text(PYTHON_PACKAGES_TOML)

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

    return tmp_path / "guest"


# ---------------------------------------------------------------------------
# parse_toml
# ---------------------------------------------------------------------------


class TestParseToml:
    def test_basic_parse(self, tmp_path):
        f = tmp_path / "test.toml"
        f.write_text('[foo]\nbar = 42\n')
        data = parse_toml(f)
        assert data["foo"]["bar"] == 42

    def test_file_not_found(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            parse_toml(tmp_path / "nonexistent.toml")

    def test_invalid_toml(self, tmp_path):
        f = tmp_path / "bad.toml"
        f.write_text("[invalid\nno closing bracket")
        with pytest.raises(tomllib.TOMLDecodeError):
            parse_toml(f)


# ---------------------------------------------------------------------------
# load_guest_config -- minimal
# ---------------------------------------------------------------------------


class TestLoadGuestConfigMinimal:
    def test_returns_guest_image_config(self, guest_minimal):
        cfg = load_guest_config(guest_minimal)
        assert isinstance(cfg, GuestImageConfig)

    def test_loads_build(self, guest_minimal):
        cfg = load_guest_config(guest_minimal)
        assert cfg.build.compression is Compression.ZSTD
        assert cfg.build.compression_level == 15

    def test_build_has_arm64(self, guest_minimal):
        cfg = load_guest_config(guest_minimal)
        assert "arm64" in cfg.build.architectures
        arch = cfg.build.architectures["arm64"]
        assert arch.docker_platform == "linux/arm64"

    def test_defaults_for_optional_sections(self, guest_minimal):
        cfg = load_guest_config(guest_minimal)
        assert cfg.ai_providers == {}
        assert cfg.package_sets == {}
        assert cfg.mcp_servers == {}
        assert cfg.web_security.allow_read is False
        assert cfg.vm_resources.cpu_count == 4
        assert cfg.vm_environment.shell.term == "xterm-256color"


# ---------------------------------------------------------------------------
# load_guest_config -- full
# ---------------------------------------------------------------------------


class TestLoadGuestConfigFull:
    def test_loads_all(self, guest_full):
        cfg = load_guest_config(guest_full)
        assert len(cfg.ai_providers) == 2
        assert len(cfg.package_sets) == 1
        assert len(cfg.mcp_servers) == 1
        assert len(cfg.web_security.search) == 1
        assert len(cfg.web_security.registry) == 1
        assert len(cfg.web_security.repository) == 1

    def test_ai_providers_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        assert "google" in cfg.ai_providers
        google = cfg.ai_providers["google"]
        assert google.name == "Google AI"
        assert google.api_key.env_vars == ["GEMINI_API_KEY"]
        assert google.install is not None
        assert google.install.manager is PackageManager.NPM
        assert "settings_json" in google.files

    def test_multiple_ai_providers(self, guest_full):
        cfg = load_guest_config(guest_full)
        assert "google" in cfg.ai_providers
        assert "anthropic" in cfg.ai_providers
        assert cfg.ai_providers["anthropic"].name == "Anthropic"

    def test_package_sets_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        assert "python" in cfg.package_sets
        ps = cfg.package_sets["python"]
        assert ps.manager is PackageManager.UV
        assert "pytest" in ps.packages
        assert ps.network is not None
        assert ps.network.name == "PyPI"

    def test_mcp_servers_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        assert "capsem" in cfg.mcp_servers
        mcp = cfg.mcp_servers["capsem"]
        assert mcp.transport is McpTransport.STDIO
        assert mcp.command == "/run/capsem-mcp-server"
        assert mcp.builtin is True

    def test_web_security_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        ws = cfg.web_security
        assert ws.custom_allow == ["elie.net", "*.elie.net"]
        assert "google" in ws.search
        assert ws.search["google"].allow_get is True
        assert "pypi" in ws.registry
        assert "github" in ws.repository

    def test_vm_resources_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        r = cfg.vm_resources
        assert r.cpu_count == 4
        assert r.ram_gb == 4
        assert r.scratch_disk_size_gb == 16

    def test_vm_environment_loaded(self, guest_full):
        cfg = load_guest_config(guest_full)
        e = cfg.vm_environment
        assert e.shell.term == "xterm-256color"
        assert e.shell.bashrc is not None
        assert e.shell.bashrc.content == "PS1='$ '"
        assert e.tls.ca_bundle == "/etc/ssl/certs/ca-certificates.crt"


# ---------------------------------------------------------------------------
# load_guest_config -- errors
# ---------------------------------------------------------------------------


class TestLoadGuestConfigErrors:
    def test_missing_build_toml(self, tmp_path):
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        with pytest.raises(FileNotFoundError):
            load_guest_config(tmp_path / "guest")

    def test_missing_config_dir(self, tmp_path):
        guest = tmp_path / "guest"
        guest.mkdir()
        with pytest.raises(FileNotFoundError):
            load_guest_config(guest)

    def test_invalid_toml_syntax(self, tmp_path):
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        (config / "build.toml").write_text("[broken\n")
        with pytest.raises(tomllib.TOMLDecodeError):
            load_guest_config(tmp_path / "guest")

    def test_invalid_model_data(self, tmp_path):
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        # compression_level out of range
        (config / "build.toml").write_text("""\
[build]
compression_level = 99

[build.architectures.arm64]
docker_platform = "linux/arm64"
rust_target = "aarch64-unknown-linux-musl"
kernel_image = "arch/arm64/boot/Image"
defconfig = "kernel/defconfig.arm64"
""")
        with pytest.raises(ValidationError):
            load_guest_config(tmp_path / "guest")

    def test_missing_required_field(self, tmp_path):
        config = tmp_path / "guest" / "config"
        config.mkdir(parents=True)
        # build.toml without architectures
        (config / "build.toml").write_text("[build]\ncompression = 'zstd'\n")
        with pytest.raises(ValidationError):
            load_guest_config(tmp_path / "guest")


# ---------------------------------------------------------------------------
# load_guest_config -- edge cases
# ---------------------------------------------------------------------------


class TestLoadGuestConfigEdgeCases:
    def test_empty_ai_directory(self, guest_minimal):
        (guest_minimal / "config" / "ai").mkdir()
        cfg = load_guest_config(guest_minimal)
        assert cfg.ai_providers == {}

    def test_non_toml_files_ignored(self, guest_minimal):
        ai = guest_minimal / "config" / "ai"
        ai.mkdir()
        (ai / "README.md").write_text("# Not a TOML file")
        (ai / "google.toml").write_text(GOOGLE_AI_TOML)
        cfg = load_guest_config(guest_minimal)
        assert len(cfg.ai_providers) == 1
        assert "google" in cfg.ai_providers

    def test_deterministic_order(self, guest_minimal):
        """Files loaded in sorted order for determinism."""
        ai = guest_minimal / "config" / "ai"
        ai.mkdir()
        (ai / "z_provider.toml").write_text(GOOGLE_AI_TOML.replace("google", "z_prov"))
        (ai / "a_provider.toml").write_text(ANTHROPIC_AI_TOML.replace("anthropic", "a_prov"))
        cfg = load_guest_config(guest_minimal)
        keys = list(cfg.ai_providers.keys())
        assert keys == sorted(keys)

    def test_multi_arch_build(self, guest_minimal):
        """build.toml with multiple architectures."""
        (guest_minimal / "config" / "build.toml").write_text("""\
[build]
compression = "gzip"
compression_level = 9

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
""")
        cfg = load_guest_config(guest_minimal)
        assert cfg.build.compression is Compression.GZIP
        assert len(cfg.build.architectures) == 2


# ---------------------------------------------------------------------------
# Helpers for JSON generator tests
# ---------------------------------------------------------------------------


def _collect_setting_ids(obj: dict, path: str = "") -> dict[str, dict]:
    """Walk the defaults.json structure and collect setting leaf IDs with their data."""
    result: dict[str, dict] = {}
    if isinstance(obj, dict):
        if "type" in obj:
            result[path] = {"type": obj["type"], "default": obj.get("default")}
            return result
        if "action" in obj:
            result[path] = {"action": obj["action"]}
            return result
        for key, val in obj.items():
            if key in ("name", "description", "enabled_by", "collapsed"):
                continue
            child_path = f"{path}.{key}" if path else key
            if isinstance(val, dict):
                result.update(_collect_setting_ids(val, child_path))
    return result


# ---------------------------------------------------------------------------
# generate_defaults_json -- structure
# ---------------------------------------------------------------------------


class TestGenerateDefaultsJsonStructure:
    """Tests for the JSON generator structure."""

    def test_returns_dict(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        assert isinstance(result, dict)

    def test_has_settings_and_mcp_keys(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        assert "settings" in result
        assert "mcp" in result

    def test_settings_has_top_level_groups(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        settings = result["settings"]
        for group in ("app", "ai", "repository", "security", "vm", "appearance"):
            assert group in settings, f"missing top-level group: {group}"

    def test_ai_provider_has_allow_setting(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        google = result["settings"]["ai"]["google"]
        assert "allow" in google
        assert google["allow"]["type"] == "bool"

    def test_ai_provider_has_apikey_setting(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        google = result["settings"]["ai"]["google"]
        assert "api_key" in google
        assert google["api_key"]["type"] == "apikey"
        assert google["api_key"]["meta"]["env_vars"] == ["GEMINI_API_KEY"]

    def test_ai_provider_has_domains_setting(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        google = result["settings"]["ai"]["google"]
        assert "domains" in google
        assert google["domains"]["type"] == "text"
        assert "*.googleapis.com" in google["domains"]["default"]

    def test_web_security_structure(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        sec = result["settings"]["security"]
        assert "web" in sec
        assert sec["web"]["allow_read"]["type"] == "bool"
        assert sec["web"]["allow_read"]["default"] is False

    def test_vm_resources_structure(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        res = result["settings"]["vm"]["resources"]
        assert res["cpu_count"]["type"] == "number"
        assert res["cpu_count"]["default"] == 4

    def test_mcp_servers(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        assert "capsem" in result["mcp"]
        assert result["mcp"]["capsem"]["transport"] == "stdio"
        assert result["mcp"]["capsem"]["command"] == "/run/capsem-mcp-server"

    def test_valid_json_roundtrip(self, guest_full):
        cfg = load_guest_config(guest_full)
        result = generate_defaults_json(cfg)
        text = json.dumps(result)
        parsed = json.loads(text)
        assert parsed == result


# ---------------------------------------------------------------------------
# generate_defaults_json -- conformance with current defaults.json
# ---------------------------------------------------------------------------


class TestGenerateDefaultsJsonConformance:
    """Verify generated JSON matches the hand-authored defaults.json."""

    @pytest.fixture
    def real_config(self):
        return load_guest_config(PROJECT_ROOT / "guest")

    @pytest.fixture
    def generated(self, real_config):
        return generate_defaults_json(real_config)

    @pytest.fixture
    def current_defaults(self):
        with open(PROJECT_ROOT / "config" / "defaults.json") as f:
            return json.load(f)

    def test_same_setting_ids(self, generated, current_defaults):
        """Every setting ID in defaults.json is in the generated JSON."""
        current_ids = set(_collect_setting_ids(current_defaults["settings"]).keys())
        gen_ids = set(_collect_setting_ids(generated["settings"]).keys())
        missing = current_ids - gen_ids
        extra = gen_ids - current_ids
        assert not missing, f"Missing setting IDs: {missing}"
        assert not extra, f"Extra setting IDs: {extra}"

    def test_same_setting_types(self, generated, current_defaults):
        """Every setting has the same type in both."""
        current = _collect_setting_ids(current_defaults["settings"])
        gen = _collect_setting_ids(generated["settings"])
        for sid, data in current.items():
            if "type" in data:
                assert gen[sid].get("type") == data["type"], \
                    f"{sid}: expected type={data['type']}, got {gen[sid].get('type')}"
            if "action" in data:
                assert gen[sid].get("action") == data["action"], \
                    f"{sid}: expected action={data['action']}, got {gen[sid].get('action')}"

    def test_same_default_values(self, generated, current_defaults):
        """Default values match between generated and hand-authored."""
        current = _collect_setting_ids(current_defaults["settings"])
        gen = _collect_setting_ids(generated["settings"])
        for sid, data in current.items():
            if "default" in data:
                assert gen[sid].get("default") == data["default"], \
                    f"{sid}: default mismatch: {data['default']!r} vs {gen[sid].get('default')!r}"

    def test_same_mcp_servers(self, generated, current_defaults):
        """MCP server definitions match."""
        assert set(generated["mcp"].keys()) == set(current_defaults["mcp"].keys())
        for key in current_defaults["mcp"]:
            for field in ("transport", "command", "builtin"):
                if field in current_defaults["mcp"][key]:
                    assert generated["mcp"][key].get(field) == current_defaults["mcp"][key][field], \
                        f"mcp.{key}.{field}: mismatch"

    def test_ai_provider_enabled_by(self, generated, current_defaults):
        """AI provider groups have correct enabled_by."""
        for key in current_defaults["settings"]["ai"]:
            if key in ("name", "description", "collapsed"):
                continue
            cur = current_defaults["settings"]["ai"][key]
            gen = generated["settings"]["ai"][key]
            if "enabled_by" in cur:
                assert gen.get("enabled_by") == cur["enabled_by"], \
                    f"ai.{key}.enabled_by: {cur['enabled_by']!r} vs {gen.get('enabled_by')!r}"

    def test_web_service_enabled_by(self, generated, current_defaults):
        """Web service groups have correct enabled_by."""
        for svc_type in ("search", "registry"):
            cur_section = current_defaults["settings"]["security"]["services"][svc_type]
            gen_section = generated["settings"]["security"]["services"][svc_type]
            for key in cur_section:
                if key in ("name", "description", "collapsed"):
                    continue
                cur = cur_section[key]
                gen = gen_section[key]
                if "enabled_by" in cur:
                    assert gen.get("enabled_by") == cur["enabled_by"], \
                        f"security.services.{svc_type}.{key}.enabled_by: mismatch"

    def test_repo_provider_enabled_by(self, generated, current_defaults):
        """Repository provider groups have correct enabled_by."""
        cur_provs = current_defaults["settings"]["repository"]["providers"]
        gen_provs = generated["settings"]["repository"]["providers"]
        for key in cur_provs:
            if key in ("name", "description", "collapsed"):
                continue
            if "enabled_by" in cur_provs[key]:
                assert gen_provs[key].get("enabled_by") == cur_provs[key]["enabled_by"]
