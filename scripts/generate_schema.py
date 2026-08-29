"""Generate settings schema, UI metadata, and frontend mock data.

`--settings-dir` exists so the checker can generate somewhere else and compare.
Writing the tracked files in place and diffing afterwards worked, but it means
every gate run writes into its own checked-in source -- invisible while the
bytes match, and refused outright once the run is sandboxed against writing to
its own tree.
"""

import argparse
import json
from pathlib import Path

from capsem_builder.image.config import (
    generate_defaults_json,
    generate_mock_ts,
    load_guest_config,
)
from capsem_builder.image.schema import export_json_schema

PROJECT_ROOT = Path(__file__).parent.parent
SCHEMA_PATH = PROJECT_ROOT / "config" / "settings" / "schema.generated.json"
DEFAULTS_PATH = PROJECT_ROOT / "config" / "settings" / "ui-metadata.generated.json"
MOCK_PATH = PROJECT_ROOT / "web" / "app" / "src" / "lib" / "mock-settings.generated.ts"
IMAGE_CONFIG_DIR = PROJECT_ROOT / "config" / "docker" / "image"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--settings-dir",
        type=Path,
        default=SCHEMA_PATH.parent,
        help="where the two tracked settings files go; defaults to the checkout's own",
    )
    parser.add_argument(
        "--mock",
        type=Path,
        default=MOCK_PATH,
        help="where the frontend mock goes. Gitignored, so it stays in the checkout even "
        "when the tracked pair is generated elsewhere -- the web checks import it.",
    )
    arguments = parser.parse_args()
    settings_dir = arguments.settings_dir
    settings_dir.mkdir(parents=True, exist_ok=True)
    schema_path = settings_dir / SCHEMA_PATH.name
    defaults_path = settings_dir / DEFAULTS_PATH.name
    mock_path = arguments.mock

    schema = export_json_schema()
    schema_path.write_text(json.dumps(schema, indent=2) + "\n")
    print(f"Wrote {schema_path}")
    print(f"  Size: {schema_path.stat().st_size} bytes")

    config = load_guest_config(IMAGE_CONFIG_DIR)
    defaults = generate_defaults_json(config)
    defaults_path.write_text(json.dumps(defaults, indent=2) + "\n")
    print(f"Wrote {defaults_path}")
    print(f"  Size: {defaults_path.stat().st_size} bytes")

    mock_ts = generate_mock_ts(defaults, mcp_tools=[])
    mock_path.parent.mkdir(parents=True, exist_ok=True)
    mock_path.write_text(mock_ts)
    print(f"Wrote {mock_path}")
    print(f"  Size: {mock_path.stat().st_size} bytes")

    # Summary
    settings = defaults.get("settings", {})
    print(f"  Settings groups: {[k for k in settings if k not in ('name','description','collapsed')]}")
    print("  MCP servers: profile routes")
    print("  MCP tools: profile routes")


if __name__ == "__main__":
    main()
