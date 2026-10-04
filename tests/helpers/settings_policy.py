"""Write policy into a test service's own settings.toml.

Policy is the built-in defaults, the user's `$CAPSEM_HOME/settings.toml` and
the corp config. A test that needs a rule writes it here: before the VM is
created, or followed by `POST /corp/reload`, which re-materializes and reloads
every running VM's policy.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

import tomli_w


def write_settings_rule(home_dir: Path, rule_id: str, **rule: Any) -> None:
    """Set one `[profiles.rules.<rule_id>]` rule; `match` is the CEL condition.

    Only that table is replaced or appended: the rest of settings.toml -- an
    `[images]` grant, a plugin mode the service wrote -- is left byte for byte.
    """
    path = Path(home_dir) / "settings.toml"
    text = path.read_text() if path.exists() else ""
    header = f"[profiles.rules.{rule_id}]"
    text = re.sub(rf"(?m)^{re.escape(header)}\n(?:[^\[\n][^\n]*\n|\n)*", "", text)
    table = tomli_w.dumps({"profiles": {"rules": {rule_id: {"name": rule_id, **rule}}}})
    separator = "\n" if text and not text.endswith("\n\n") else ""
    if text and not text.endswith("\n"):
        separator = "\n\n"
    path.write_text(text + separator + table)


def reload_policy(client: Any) -> dict[str, Any]:
    """Put the current settings.toml in force in every running VM."""
    response = client.post("/corp/reload", {}, timeout=30)
    assert response.get("success") is True, response
    return response


def apply_settings_rule(service: Any, rule_id: str, **rule: Any) -> dict[str, Any]:
    """Write one rule into the service's settings.toml and reload running VMs."""
    write_settings_rule(service.home_dir, rule_id, **rule)
    return reload_policy(service.client())
