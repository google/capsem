"""Fast, sparse VM boot does not wipe guest heap pages eagerly."""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
GUEST_BOOT_MEMORY_RATIONALE = """\
Guest heap wiping is explicitly disabled for fast, sparse VM startup.
Wiping 12 GiB took a minute before userspace. Fresh anonymous host memory
already starts zeroed; guest alloc/free wiping must not touch all RAM at boot.
Explicit zeros override CONFIG_INIT_ON_ALLOC_DEFAULT_ON in both kernels.
"""


def assert_wiping_disabled(flags: str) -> None:
    for name in ("init_on_alloc", "init_on_free"):
        values = re.findall(rf"(?<!\S){name}=([^\s\\]+)", flags)
        assert values == ["0"], f"{name} values {values}: {GUEST_BOOT_MEMORY_RATIONALE}"


def test_the_real_kernel_flags_disable_guest_heap_wiping():
    source = (ROOT / "crates/capsem-core/src/vm/config.rs").read_text()
    macro = source.split("macro_rules! guest_kernel_flags", 1)[1].split("};", 1)[0]
    assert_wiping_disabled(macro.replace('"', " "))


@pytest.mark.parametrize(
    "flags",
    [
        "init_on_alloc=1 init_on_free=0",
        "init_on_alloc=0 init_on_free=1",
        "init_on_free=0",
        "init_on_alloc=0",
        "init_on_alloc=0 init_on_alloc=1 init_on_free=0",
    ],
)
def test_missing_enabled_or_overridden_wiping_flags_are_refused(flags):
    with pytest.raises(AssertionError, match="Guest heap wiping"):
        assert_wiping_disabled(flags)


def test_explicit_zero_flags_are_accepted():
    assert_wiping_disabled("root=/dev/vda ro init_on_alloc=0 init_on_free=0 slab_nomerge")
