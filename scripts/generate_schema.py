"""Generate settings-schema.json, defaults.json, and mock-data.generated.ts."""

import json
from pathlib import Path

from capsem.builder.config import (
    generate_defaults_json,
    generate_mock_ts,
    load_guest_config,
)
from capsem.builder.schema import export_json_schema

PROJECT_ROOT = Path(__file__).parent.parent
SCHEMA_PATH = PROJECT_ROOT / "config" / "settings-schema.json"
DEFAULTS_PATH = PROJECT_ROOT / "config" / "defaults.json"
MCP_TOOLS_PATH = PROJECT_ROOT / "config" / "mcp-tools.json"
MOCK_PATH = PROJECT_ROOT / "frontend" / "src" / "lib" / "mock-settings.generated.ts"
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

    # Load MCP tool defs exported by mcp_export binary
    mcp_tools = json.loads(MCP_TOOLS_PATH.read_text()) if MCP_TOOLS_PATH.exists() else []

    mock_ts = generate_mock_ts(defaults, mcp_tools=mcp_tools)
    MOCK_PATH.write_text(mock_ts)
    print(f"Wrote {MOCK_PATH}")
    print(f"  Size: {MOCK_PATH.stat().st_size} bytes")

    # Summary
    settings = defaults.get("settings", {})
    mcp_servers = defaults.get("mcp", {})
    print(f"  Settings groups: {[k for k in settings if k not in ('name','description','collapsed')]}")
    print(f"  MCP servers: {list(mcp_servers.keys())}")
    print(f"  MCP tools: {len(mcp_tools)}")


if __name__ == "__main__":
    main()
