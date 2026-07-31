"""`python -m capsem.gate`, for callers without the console script on PATH."""

from __future__ import annotations

from .cli import main


raise SystemExit(main())
