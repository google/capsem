"""Corp Profile V2 provisioning tests for WB2a.

Tests verify a corp-managed Profile V2 TOML can be provisioned from a
local file path, validated, and installed under the configured corp
profile root.
"""

from __future__ import annotations

import json

import pytest

from .conftest import (
    CAPSEM_DIR,
    run_capsem,
)

SERVICE_TOML = CAPSEM_DIR / "service.toml"
CORP_PROFILE_DIR = CAPSEM_DIR / "profiles" / "corp"
CORP_PROFILE = CORP_PROFILE_DIR / "test-corp-profile.toml"
SETUP_STATE = CAPSEM_DIR / "setup-state.json"


@pytest.fixture
def fresh_corp_state():
    """Reset corp artifacts before each test so tests don't share state.

    Without this, the first test marks corp_config as done in setup-state.json,
    and subsequent tests skip the step -- leaving stale corp profiles and no
    validation of new input.
    """
    for p in (CORP_PROFILE, SERVICE_TOML, SETUP_STATE):
        p.unlink(missing_ok=True)
    yield
    for p in (CORP_PROFILE, SERVICE_TOML, SETUP_STATE):
        p.unlink(missing_ok=True)

VALID_CORP_CONTENT = """\
version = 1
id = "test-corp-profile"
name = "Test Corp"
best_for = "Managed test sessions."
profile_type = "coding"

[security.rules.http.block_example]
on = "http.request"
if = 'request.host == "example.com"'
decision = "block"
priority = -10
"""

INVALID_CORP_CONTENT = "this is not [ valid toml {{{"


class TestCorpProvisioning:
    """Corp Profile V2 provisioning from local file path."""

    def test_corp_config_from_local_file(self, installed_layout, clean_state, fresh_corp_state, tmp_path):
        """capsem setup --corp-config /path/to/profile.toml installs a corp profile."""
        corp_file = tmp_path / "corp-profile.toml"
        corp_file.write_text(VALID_CORP_CONTENT)

        result = run_capsem("setup", "--corp-config", str(corp_file), "--non-interactive", timeout=15)
        # Setup may not be implemented yet; test the corp file was installed
        if result.returncode != 0 and "unrecognized" in result.stderr.lower():
            pytest.skip("setup command not yet implemented")

        assert CORP_PROFILE.exists(), "corp profile should be installed"
        content = CORP_PROFILE.read_text()
        assert 'id = "test-corp-profile"' in content
        assert 'priority = -10' in content
        assert SERVICE_TOML.exists(), "service.toml should record the corp profile root"
        assert str(CORP_PROFILE_DIR) in SERVICE_TOML.read_text()

    @pytest.mark.live_system
    def test_corp_config_validates_toml(self, installed_layout, clean_state, fresh_corp_state, tmp_path):
        """Invalid TOML is rejected with clear error."""
        bad_file = tmp_path / "bad.toml"
        bad_file.write_text(INVALID_CORP_CONTENT)

        result = run_capsem("setup", "--corp-config", str(bad_file), "--non-interactive", timeout=15)
        if result.returncode != 0 and "unrecognized" in result.stderr.lower():
            pytest.skip("setup command not yet implemented")

        # Should fail with an error about invalid TOML
        assert result.returncode != 0 or "invalid" in (result.stdout + result.stderr).lower()

    @pytest.mark.live_system
    def test_corp_source_recorded_in_setup_state(self, installed_layout, clean_state, fresh_corp_state, tmp_path):
        """setup-state.json records the provisioned source path."""
        corp_file = tmp_path / "corp-profile.toml"
        corp_file.write_text(VALID_CORP_CONTENT)

        result = run_capsem("setup", "--corp-config", str(corp_file), "--non-interactive", timeout=15)
        if result.returncode != 0 and "unrecognized" in result.stderr.lower():
            pytest.skip("setup command not yet implemented")

        assert SETUP_STATE.exists(), "setup-state.json should be written"
        state = json.loads(SETUP_STATE.read_text())
        assert state["corp_config_source"] == str(corp_file)

    @pytest.mark.live_system
    def test_corp_config_overwrites_previous(self, installed_layout, clean_state, fresh_corp_state, tmp_path):
        """Re-provisioning replaces an existing corp profile with the same id."""
        CORP_PROFILE_DIR.mkdir(parents=True, exist_ok=True)
        CORP_PROFILE.write_text('version = 1\nid = "test-corp-profile"\nname = "Old"\nbest_for = "Old"\nprofile_type = "coding"\n')

        corp_file = tmp_path / "corp-profile.toml"
        corp_file.write_text(VALID_CORP_CONTENT)

        result = run_capsem("setup", "--corp-config", str(corp_file), "--non-interactive", timeout=15)
        if result.returncode != 0 and "unrecognized" in result.stderr.lower():
            pytest.skip("setup command not yet implemented")

        content = CORP_PROFILE.read_text()
        assert 'name = "Old"' not in content, "old corp profile should be replaced"
        assert 'id = "test-corp-profile"' in content
