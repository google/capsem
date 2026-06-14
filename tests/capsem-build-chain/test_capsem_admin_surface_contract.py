"""capsem-admin exposes one profile-derived rail, not authoring shortcuts."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_capsem_admin_has_no_scaffold_or_init_helpers() -> None:
    source = (PROJECT_ROOT / "crates/capsem-admin/src/main.rs").read_text()

    forbidden = [
        "PRIMARY_PROFILE_TEMPLATE",
        "ProfileInitArgs",
        "InitArgs",
        "init_file_command",
        "init_profile_command",
    ]
    failures = [needle for needle in forbidden if needle in source]

    assert not failures, "capsem-admin scaffold helpers must stay burned: " + ", ".join(
        failures
    )
