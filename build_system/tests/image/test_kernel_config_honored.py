"""The kernel build refuses a defconfig whose pins the kernel did not honor.

Kconfig drops a misspelled, renamed or unsatisfiable symbol without a word.
`SLAB_FREELIST_RANDOMIZE` (the symbol is `SLAB_FREELIST_RANDOM`) left slab
freelist randomization off on both architectures while the hardening docs
listed it as on, and 6.9's `MITIGATION_*` rename emptied two x86 pins.

These tests run the template's own check, the exact shell Docker runs, over
fixture configs.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

TEMPLATE = Path(__file__).resolve().parents[3] / "config" / "docker" / "Dockerfile.kernel.j2"
DEFCONFIG = "/tmp/capsem.defconfig"


def check_script() -> str:
    """The verification RUN, joined the way Docker joins continuation lines."""
    text = TEMPLATE.read_text().replace("\\\n", " ")
    [run] = [line for line in text.splitlines() if line.startswith("RUN unhonored=")]
    return run.removeprefix("RUN ")


def verify(tmp_path: Path, defconfig: str, effective: str) -> subprocess.CompletedProcess[str]:
    pinned = tmp_path / "capsem.defconfig"
    pinned.write_text(defconfig)
    (tmp_path / ".config").write_text(effective)
    script = check_script()
    assert DEFCONFIG in script, "the check must read the defconfig the build copied"
    return subprocess.run(
        ["sh", "-c", script.replace(DEFCONFIG, str(pinned))],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )


def test_honored_pins_pass(tmp_path: Path) -> None:
    result = verify(
        tmp_path,
        '# comment\nCONFIG_A=y\nCONFIG_B=n\nCONFIG_C=48\nCONFIG_D="x y"\n',
        'CONFIG_A=y\n# CONFIG_B is not set\nCONFIG_C=48\nCONFIG_D="x y"\nCONFIG_E=y\n',
    )
    assert result.returncode == 0, result.stderr


def test_a_disabled_pin_that_is_absent_counts_as_honored(tmp_path: Path) -> None:
    result = verify(tmp_path, "CONFIG_GONE=n\n", "CONFIG_A=y\n")
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    ("defconfig", "effective", "named"),
    [
        # A symbol that does not exist: the SLAB_FREELIST_RANDOMIZE case.
        (
            "CONFIG_SLAB_FREELIST_RANDOMIZE=y\n",
            "# CONFIG_SLAB_FREELIST_RANDOM is not set\n",
            "CONFIG_SLAB_FREELIST_RANDOMIZE=y",
        ),
        # A choice Kconfig resolved differently: arm64 VA bits.
        ("CONFIG_ARM64_VA_BITS=48\n", "CONFIG_ARM64_VA_BITS=52\n", "CONFIG_ARM64_VA_BITS=48"),
        # An option forced on despite a pin to off.
        ("CONFIG_IO_URING=n\n", "CONFIG_IO_URING=y\n", "CONFIG_IO_URING=n"),
    ],
)
def test_an_unhonored_pin_fails_the_build_and_is_named(
    tmp_path: Path, defconfig: str, effective: str, named: str
) -> None:
    result = verify(tmp_path, f"CONFIG_OK=y\n{defconfig}", f"CONFIG_OK=y\n{effective}")
    assert result.returncode != 0
    assert named in result.stderr
    assert "CONFIG_OK" not in result.stderr
