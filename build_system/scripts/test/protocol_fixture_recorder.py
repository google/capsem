"""Record and optionally replay sanitized protocol fixtures."""

import sys
from importlib import import_module
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))


def main():
    return import_module("build_system.tests.helpers.protocol_fixture_recorder").main()

if __name__ == "__main__":
    raise SystemExit(main())
