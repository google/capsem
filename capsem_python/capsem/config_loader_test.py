# Copyright 2025 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Tests for config_loader module"""

import pytest
import tempfile
from pathlib import Path

from .config_loader import load_policies_from_directory, load_policy_class
from .policies.debug_policy import DebugPolicy
from .policies.pii_policy import PIIPolicy


def test_load_policy_class_debug():
    """Test loading DebugPolicy class"""
    policy_class = load_policy_class("capsem.policies.debug_policy.DebugPolicy")
    assert policy_class == DebugPolicy


def test_load_policy_class_pii():
    """Test loading PIIPolicy class"""
    policy_class = load_policy_class("capsem.policies.pii_policy.PIIPolicy")
    assert policy_class == PIIPolicy


def test_load_policy_class_invalid():
    """Test loading invalid policy class"""
    with pytest.raises((ImportError, AttributeError)):
        load_policy_class("capsem.policies.nonexistent.FakePolicy")


def test_load_policies_from_directory_debug():
    """Test loading debug policy from config directory"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = true\n")

        manager = load_policies_from_directory(config_dir)

        assert len(manager.policies) == 1
        assert isinstance(manager.policies[0], DebugPolicy)
        assert manager.policies[0].name == "Debug"


def test_load_policies_from_directory_disabled():
    """Test that disabled policies are not loaded"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml with enabled = false
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = false\n")

        manager = load_policies_from_directory(config_dir)

        # Should fall back to default DebugPolicy since no policies loaded
        assert len(manager.policies) == 1
        assert isinstance(manager.policies[0], DebugPolicy)


def test_load_policies_from_directory_multiple():
    """Test loading multiple policies"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = true\n")

        # Create pii.toml
        pii_config = config_dir / "pii.toml"
        pii_config.write_text("""enabled = true

[entity_decisions]
EMAIL_ADDRESS = "BLOCK"
CREDIT_CARD = "CONFIRM"
""")

        manager = load_policies_from_directory(config_dir)

        assert len(manager.policies) == 2
        policy_names = [p.name for p in manager.policies]
        assert "Debug" in policy_names
        assert "PIIDetection" in policy_names


def test_load_policies_from_directory_empty():
    """Test loading from empty directory uses default DebugPolicy"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        manager = load_policies_from_directory(config_dir)

        # Should use default DebugPolicy
        assert len(manager.policies) == 1
        assert isinstance(manager.policies[0], DebugPolicy)


def test_load_policies_from_directory_nonexistent():
    """Test loading from nonexistent directory raises error"""
    with pytest.raises(FileNotFoundError):
        load_policies_from_directory("/nonexistent/directory")


def test_load_policies_from_directory_not_dir():
    """Test loading from file (not directory) raises error"""
    with tempfile.NamedTemporaryFile() as tmpfile:
        with pytest.raises(ValueError):
            load_policies_from_directory(tmpfile.name)


def test_load_policies_unknown_config_file():
    """Test that unknown .toml files are skipped"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create unknown.toml
        unknown_config = config_dir / "unknown.toml"
        unknown_config.write_text("enabled = true\n")

        manager = load_policies_from_directory(config_dir)

        # Should fall back to default DebugPolicy
        assert len(manager.policies) == 1
        assert isinstance(manager.policies[0], DebugPolicy)


def test_pii_policy_from_config_file():
    """Test loading PII policy from config file"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_file = Path(tmpdir) / "pii.toml"
        config_file.write_text("""enabled = true
check_prompts = true
check_responses = false
score_threshold = 0.7

[entity_decisions]
EMAIL_ADDRESS = "BLOCK"
CREDIT_CARD = "CONFIRM"
PHONE_NUMBER = "LOG"
""")

        policy = PIIPolicy.from_config_file(config_file)

        assert policy is not None
        assert policy.name == "PIIDetection"
        assert policy.check_prompts is True
        assert policy.check_responses is False
        assert policy.score_threshold == 0.7
        assert len(policy.entity_decisions) == 3


def test_debug_policy_from_config_file():
    """Test loading Debug policy from config file"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_file = Path(tmpdir) / "debug.toml"
        config_file.write_text("enabled = true\n")

        policy = DebugPolicy.from_config_file(config_file)

        assert policy is not None
        assert policy.name == "Debug"


def test_policy_from_config_file_disabled():
    """Test that disabled policy returns None"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_file = Path(tmpdir) / "debug.toml"
        config_file.write_text("enabled = false\n")

        policy = DebugPolicy.from_config_file(config_file)

        assert policy is None
