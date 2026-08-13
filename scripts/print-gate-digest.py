#!/usr/bin/env python3
"""Print the gate digest for an agent session, and never fail doing it.

Wired as a session-start hook, so it runs before anybody has asked for
anything. That imposes three rules it would not otherwise have.

It must not invoke the gate. `capsem-gate runs digest` is the obvious
implementation and the wrong one: it takes the history lock, it can block
behind a running gate, and a session that hangs on startup is a session nobody
debugs -- they delete the hook. This reads the file the gate already wrote.

It must not need the project's environment. Bare `python3` has no pydantic, so
importing `capsem.gate.config` fails on exactly the machines where a hook is
least welcome to be picky. The only thing needed from config is one path, and
`tomllib` is in the standard library.

And it must not fail. A missing digest is the ordinary state of a fresh
checkout, not an error. It says what is missing and returns success.

`tests/citadel/test_run_digest_echo.py` proves this stays wired to the same key
the gate writes through.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
#: The one setting this needs, named the way the schema names it.
DIGEST_PATH = ("runlog", "digest", "path")


def digest_path(root: Path) -> Path | None:
    """Where the gate writes the digest, according to config."""
    settings = tomllib.loads((root / "config" / "gate.toml").read_text(encoding="utf-8"))
    for key in DIGEST_PATH:
        if not isinstance(settings, dict) or key not in settings:
            return None
        settings = settings[key]
    return root / settings if isinstance(settings, str) else None


def main() -> int:
    try:
        target = digest_path(ROOT)
    except (OSError, ValueError) as error:
        print(f"gate digest unavailable ({type(error).__name__}: {error})")
        return 0

    if target is None:
        print(f"gate digest unavailable: config/gate.toml has no {'.'.join(DIGEST_PATH)}")
        return 0
    if not target.is_file():
        print(
            "No gate digest yet. Every `just test` writes one, or run "
            "`uv run capsem-gate runs digest`."
        )
        return 0

    print(target.read_text(encoding="utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
