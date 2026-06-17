from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_justfile_does_not_expose_legacy_guest_dir_knob() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "--guest-dir" not in justfile
    assert "capsem-builder build guest" not in justfile
    assert "capsem-builder agent config/docker/image" in justfile
    assert "capsem-builder agent --arch" not in justfile
