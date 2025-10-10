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

"""Tests for policy config loading in the proxy"""

import os
import tempfile
import pytest
from pathlib import Path
from fastapi.testclient import TestClient


def test_proxy_starts_with_default_policy():
    """Test that proxy starts with default DebugPolicy when no config provided"""
    # Don't set PROXY_CONFIG_DIR
    if "PROXY_CONFIG_DIR" in os.environ:
        del os.environ["PROXY_CONFIG_DIR"]

    # Import server (will initialize with default policy)
    from capsem_proxy.server import app
    from capsem_proxy.capsem_integration import security_manager

    client = TestClient(app)

    # Verify proxy is running
    response = client.get("/health")
    assert response.status_code == 200

    # Verify default policy is loaded
    assert len(security_manager.policies) == 1
    assert security_manager.policies[0].name == "Debug"


def test_proxy_loads_debug_policy_from_config():
    """Test that proxy loads debug policy from config directory"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = true\n")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload the module to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        from capsem_proxy.capsem_integration import security_manager

        # Verify debug policy is loaded
        assert len(security_manager.policies) == 1
        assert security_manager.policies[0].name == "Debug"


def test_proxy_loads_pii_policy_from_config():
    """Test that proxy loads PII policy from config directory"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create pii.toml
        pii_config = config_dir / "pii.toml"
        pii_config.write_text("""enabled = true
check_prompts = true

[entity_decisions]
EMAIL_ADDRESS = "BLOCK"
CREDIT_CARD = "CONFIRM"
""")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload the module to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        from capsem_proxy.capsem_integration import security_manager

        # Verify PII policy is loaded
        assert len(security_manager.policies) == 1
        assert security_manager.policies[0].name == "PIIDetection"


def test_proxy_loads_multiple_policies_from_config():
    """Test that proxy loads multiple policies from config directory"""
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
""")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload the module to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        from capsem_proxy.capsem_integration import security_manager

        # Verify both policies are loaded
        assert len(security_manager.policies) == 2
        policy_names = [p.name for p in security_manager.policies]
        assert "Debug" in policy_names
        assert "PIIDetection" in policy_names


def test_proxy_with_disabled_policy():
    """Test that proxy skips disabled policies"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml with enabled = false
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = false\n")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload the module to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        from capsem_proxy.capsem_integration import security_manager

        # Should fall back to default DebugPolicy
        assert len(security_manager.policies) == 1
        assert security_manager.policies[0].name == "Debug"


def test_proxy_blocks_request_with_pii_policy():
    """Test that proxy blocks requests based on PII policy config"""
    from unittest.mock import AsyncMock, MagicMock, patch

    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create pii.toml that blocks email addresses
        pii_config = config_dir / "pii.toml"
        pii_config.write_text("""enabled = true

[entity_decisions]
EMAIL_ADDRESS = "BLOCK"
""")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload modules to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        # Import server after config is loaded
        import capsem_proxy.server
        importlib.reload(capsem_proxy.server)
        from capsem_proxy.server import app

        client = TestClient(app)

        # Mock httpx so we don't need real API
        with patch("capsem_proxy.providers.openai.httpx.AsyncClient") as mock_httpx:
            mock_client = AsyncMock()
            mock_client.__aenter__.return_value = mock_client
            mock_client.__aexit__.return_value = None
            mock_httpx.return_value = mock_client

            # Try to send a request with an email address
            response = client.post(
                "/v1/chat/completions",
                headers={"Authorization": "Bearer sk-test-key"},
                json={
                    "model": "gpt-4",
                    "messages": [
                        {"role": "user", "content": "Contact me at test@example.com"}
                    ]
                }
            )

            # Should be blocked by PII policy before reaching OpenAI
            assert response.status_code == 403
            assert "blocked" in response.json()["detail"].lower()

            # Verify httpx was NOT called (blocked by policy)
            mock_client.post.assert_not_called()


def test_proxy_config_with_nonexistent_directory():
    """Test that proxy falls back to default when config dir doesn't exist"""
    os.environ["PROXY_CONFIG_DIR"] = "/nonexistent/directory"

    # Need to reload the module to pick up new config
    import importlib
    import capsem_proxy.capsem_integration
    importlib.reload(capsem_proxy.capsem_integration)

    from capsem_proxy.capsem_integration import security_manager

    # Should fall back to default DebugPolicy
    assert len(security_manager.policies) == 1
    assert security_manager.policies[0].name == "Debug"


def test_policy_string_representation():
    """Test that policies can be printed nicely"""
    with tempfile.TemporaryDirectory() as tmpdir:
        config_dir = Path(tmpdir)

        # Create debug.toml
        debug_config = config_dir / "debug.toml"
        debug_config.write_text("enabled = true\n")

        # Set config directory
        os.environ["PROXY_CONFIG_DIR"] = str(config_dir)

        # Need to reload the module to pick up new config
        import importlib
        import capsem_proxy.capsem_integration
        importlib.reload(capsem_proxy.capsem_integration)

        from capsem_proxy.capsem_integration import security_manager

        # Verify string representation works
        policy = security_manager.policies[0]
        assert str(policy) == "Debug"
        assert "Debug" in repr(policy)
        assert "Elie Bursztein" in repr(policy)
