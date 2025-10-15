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

"""Configuration loader for CAPSEM policies

Scans a config directory for TOML files and loads policies using their
from_config_file() class methods.
"""

import logging
from pathlib import Path

from .manager import SecurityManager

logger = logging.getLogger(__name__)


# Policy class mapping: filename -> policy class
POLICY_CLASSES = {
    "debug": "capsem.policies.debug_policy.DebugPolicy",
    "pii": "capsem.policies.pii_policy.PIIPolicy",
}


def load_policy_class(class_path: str):
    """Dynamically import and return a policy class

    Args:
        class_path: Full module path to policy class (e.g., "capsem.policies.pii_policy.PIIPolicy")

    Returns:
        Policy class

    Raises:
        ImportError: If module or class cannot be imported
    """
    module_path, class_name = class_path.rsplit(".", 1)
    module = __import__(module_path, fromlist=[class_name])
    return getattr(module, class_name)


def load_policies_from_directory(config_dir: str | Path) -> SecurityManager:
    """Load policies from TOML config files in a directory

    Scans the config directory for .toml files and loads each policy using
    its from_config_file() class method. The filename (without .toml) determines
    which policy class to use.

    Args:
        config_dir: Path to directory containing policy config files
                   (e.g., config/debug.toml, config/pii.toml)

    Returns:
        SecurityManager with loaded policies

    Raises:
        FileNotFoundError: If config directory doesn't exist
        ValueError: If config directory path is not a directory

    Example directory structure:
        config/
          ├── debug.toml      # DebugPolicy config
          └── pii.toml        # PIIPolicy config

    Example config file (config/pii.toml):
        enabled = true

        [entity_decisions]
        EMAIL_ADDRESS = "BLOCK"
        CREDIT_CARD = "CONFIRM"
        PERSON = "LOG"

        check_prompts = true
        check_responses = true
        score_threshold = 0.5
    """
    config_dir = Path(config_dir)

    if not config_dir.exists():
        raise FileNotFoundError(f"Config directory not found: {config_dir}")

    if not config_dir.is_dir():
        raise ValueError(f"Config path is not a directory: {config_dir}")

    manager = SecurityManager()
    loaded_policies = []

    # Find all .toml files in config directory
    config_files = sorted(config_dir.glob("*.toml"))

    if not config_files:
        logger.warning(f"No .toml config files found in {config_dir}")
        logger.info("Using default DebugPolicy")
        from capsem.policies.debug_policy import DebugPolicy
        manager.add_policy(DebugPolicy())
        return manager

    # Load each policy config file
    for config_file in config_files:
        policy_name = config_file.stem  # filename without .toml extension

        # Check if we have a registered policy class for this filename
        if policy_name not in POLICY_CLASSES:
            logger.warning(f"Unknown policy config file: {config_file.name}, skipping")
            logger.warning(f"Known policy types: {list(POLICY_CLASSES.keys())}")
            continue

        # Load the policy class
        class_path = POLICY_CLASSES[policy_name]
        try:
            policy_class = load_policy_class(class_path)
        except ImportError as e:
            logger.error(f"Failed to import {class_path}: {e}")
            logger.error(f"Skipping {config_file.name}")
            continue

        # Create policy instance from config file
        try:
            policy = policy_class.from_config_file(config_file)

            # Policy can return None if disabled via "enabled = false"
            if policy is None:
                logger.info(f"Policy '{policy_name}' is disabled, skipping")
                continue

            manager.add_policy(policy)
            loaded_policies.append(policy_name)
            logger.info(f"Loaded policy: {policy_name} from {config_file.name}")

        except ImportError as e:
            logger.error(f"Failed to load {policy_name}: {e}")
            logger.error(f"Install required dependencies and try again")
        except ValueError as e:
            logger.error(f"Invalid config in {config_file.name}: {e}")
            raise
        except Exception as e:
            logger.error(f"Failed to create policy from {config_file.name}: {e}")
            raise

    if not loaded_policies:
        logger.warning("No policies were loaded, using default DebugPolicy")
        from capsem.policies.debug_policy import DebugPolicy
        manager.add_policy(DebugPolicy())
    else:
        logger.info(f"Successfully loaded {len(loaded_policies)} policies: {', '.join(loaded_policies)}")

    return manager
