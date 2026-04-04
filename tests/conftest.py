"""Root conftest: ensures tests/ is on sys.path so helpers/ is importable."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
