"""Run the settings-injection integration proof."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from build_system.tests.helpers.injection_test import main  # noqa: E402

if __name__ == "__main__":
    main()
