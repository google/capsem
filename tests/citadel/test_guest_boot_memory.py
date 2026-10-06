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


PAGE_METADATA_RATIONALE = """\
Large x86 guests initialize page metadata on their available CPUs.
Single-threaded metadata initialization took about one second for 12 GiB
before CPU setup. The kernel's sparse/deferred metadata initialization
parallelizes this work without guest heap wiping, host prefaulting or THP.
The ordinary kernel build must honor every pin and native boot must prove it.
"""


def assert_parallel_page_metadata(source: str) -> None:
    for option in ("64BIT", "SMP", "SPARSEMEM", "DEFERRED_STRUCT_PAGE_INIT"):
        values = re.findall(rf"^CONFIG_{option}=(.*)$", source, re.MULTILINE)
        assert values == ["y"], f"CONFIG_{option} values {values}: {PAGE_METADATA_RATIONALE}"


def test_x86_large_guests_parallelize_page_metadata_initialization():
    source = (ROOT / "config/docker/image/kernel/defconfig.x86_64").read_text()
    assert_parallel_page_metadata(source)


@pytest.mark.parametrize("mutation", ["", "CONFIG_SMP=n\n", "CONFIG_SMP=y\n"])
def test_missing_disabled_or_duplicate_metadata_pins_are_refused(mutation):
    pins = "CONFIG_64BIT=y\nCONFIG_SMP=y\nCONFIG_SPARSEMEM=y\nCONFIG_DEFERRED_STRUCT_PAGE_INIT=y\n"
    source = pins.replace("CONFIG_SMP=y\n", mutation) if mutation != "CONFIG_SMP=y\n" else pins + mutation
    with pytest.raises(AssertionError, match="Large x86 guests"):
        assert_parallel_page_metadata(source)


def test_parallel_metadata_pins_are_accepted():
    assert_parallel_page_metadata("CONFIG_64BIT=y\nCONFIG_SMP=y\nCONFIG_SPARSEMEM=y\nCONFIG_DEFERRED_STRUCT_PAGE_INIT=y\n")
