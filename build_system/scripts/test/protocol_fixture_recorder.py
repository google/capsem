"""Record and optionally replay sanitized protocol fixtures."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from build_system.tests.helpers.protocol_fixture_recorder import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
