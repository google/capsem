from __future__ import annotations

import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILE_ROOT = PROJECT_ROOT / "config" / "profiles"


def test_checked_in_source_profiles_do_not_carry_generated_pins() -> None:
    profile_paths = sorted(PROFILE_ROOT.glob("*/profile.toml"))
    assert profile_paths, "expected at least one checked-in profile"

    forbidden = re.compile(r'^\s*(hash|size)\s=', re.MULTILINE)
    offenders = [
        str(path.relative_to(PROJECT_ROOT))
        for path in profile_paths
        if forbidden.search(path.read_text())
    ]

    assert not offenders, (
        "source profiles must not carry generated hash/size pins; "
        "materialize pins into target/config with capsem-admin: "
        + ", ".join(offenders)
    )
