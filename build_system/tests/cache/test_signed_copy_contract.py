"""`run_signed.sh` and Cargo retention must agree on what a signed copy is.

The script publishes a signed copy beside each binary it runs; retention can
reclaim only the names it recognises. When the two disagreed, 1,610 copies of
`debug/capsem-admin` (71 GB) sat outside every generation and enforcement
refused every compile on the machine.
"""

import re
from pathlib import Path

from capsem_builder.cache.cargounits import _SIGNED_COPY, SIGNED_RECEIPTS

SCRIPT = Path(__file__).resolve().parents[2] / "packaging" / "macos" / "run_signed.sh"
KEY = "0123456789abcdef" * 4


def assignment(name: str) -> str:
    [value] = re.findall(rf'^\s*{name}="([^"]+)"', SCRIPT.read_text(), flags=re.MULTILINE)
    return value


def test_published_copy_and_its_staging_file_are_recognised() -> None:
    published = assignment("published")
    assert published == '$binary_dir/.run-signed-${original##*/}-$key', (
        "run_signed.sh changed how it names signed copies: update "
        "cargounits._SIGNED_COPY and this contract together"
    )
    name = published.removeprefix("$binary_dir/").replace("${original##*/}", "capsem-admin")
    name = name.replace("$key", KEY)
    staging = assignment("staging").replace("$published", name).replace("$$", "4242")
    for published_name in (name, staging):
        found = _SIGNED_COPY.fullmatch(published_name)
        assert found is not None, f"retention does not recognise {published_name}"
        assert found.group(1) == KEY


def test_receipts_live_where_retention_looks_for_them() -> None:
    assert assignment("SIGN_CACHE_DIR") == f"$binary_dir/{SIGNED_RECEIPTS}"
    assert assignment("receipt") == "$SIGN_CACHE_DIR/$key"
