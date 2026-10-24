#!/usr/bin/env python3
"""Prepare the explicit native-only Guest FileContent acceptance executable."""

from pathlib import Path

from capsem_builder.gate.tools.native_file_content import main

if __name__ == "__main__":
    raise SystemExit(main(Path(__file__).resolve().parents[3]))
