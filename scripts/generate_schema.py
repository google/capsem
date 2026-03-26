"""Generate config/settings-schema.json and config/defaults.json from Pydantic models."""

import json
from pathlib import Path

from capsem.builder.config import generate_defaults_json, load_guest_config
from capsem.builder.schema import export_json_schema

PROJECT_ROOT = Path(__file__).parent.parent
SCHEMA_PATH = PROJECT_ROOT / "config" / "settings-schema.json"
DEFAULTS_PATH = PROJECT_ROOT / "config" / "defaults.json"
GUEST_DIR = PROJECT_ROOT / "guest"


def main():
    schema = export_json_schema()
    SCHEMA_PATH.write_text(json.dumps(schema, indent=2) + "\n")
    print(f"Wrote {SCHEMA_PATH}")
    print(f"  Size: {SCHEMA_PATH.stat().st_size} bytes")

    config = load_guest_config(GUEST_DIR)
    defaults = generate_defaults_json(config)
    DEFAULTS_PATH.write_text(json.dumps(defaults, indent=2) + "\n")
    print(f"Wrote {DEFAULTS_PATH}")
    print(f"  Size: {DEFAULTS_PATH.stat().st_size} bytes")

    # Summary
    settings = defaults.get("settings", {})
    mcp = defaults.get("mcp", {})
    print(f"  Top-level groups: {[k for k in settings if k not in ('name','description','collapsed')]}")
    print(f"  MCP servers: {list(mcp.keys())}")


if __name__ == "__main__":
    main()
