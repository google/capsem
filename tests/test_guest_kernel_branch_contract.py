"""The guest kernel assertion must track the branch the image is built from.

`config/docker/image/build.toml` decides which kernel branch Capsem builds,
and a guest diagnostic asserts the booted kernel is that branch. Those two
facts live in different languages, on different sides of the VM boundary, and
nothing but this test makes them agree.

They have already drifted once: moving the pin from 7.0 to 6.18 left the
diagnostic demanding major >= 7, so every freshly built image failed its own
diagnostics -- and the failure surfaced at the end of a release gate, minutes
of VM boots away from the one-line cause.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
BUILD_CONFIG = PROJECT_ROOT / "config" / "docker" / "image" / "build.toml"
GUEST_DIAGNOSTIC = (
    PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_environment.py"
)


def _configured_kernel_branches() -> set[str]:
    """Every `kernel_branch` in the build config, at whatever depth.

    The keys sit under `build.architectures.<arch>`; walking rather than
    indexing means adding an architecture cannot silently escape this contract.
    """
    branches: set[str] = set()

    def walk(table: dict) -> None:
        for key, value in table.items():
            if isinstance(value, dict):
                walk(value)
            elif key == "kernel_branch":
                branches.add(value)

    walk(tomllib.loads(BUILD_CONFIG.read_text(encoding="utf-8")))
    assert branches, f"no kernel_branch found in {BUILD_CONFIG}"
    return branches


def _kernel_assertion_code() -> str:
    """The kernel diagnostic's body, comments stripped.

    Comments are removed because this contract asserts what the *code* does,
    and prose explaining a rejected pattern must not read as that pattern.
    """
    source = GUEST_DIAGNOSTIC.read_text(encoding="utf-8")
    body = source.split("def test_kernel_is_supported_custom_build", maxsplit=1)[1]
    body = body.split("\ndef ", maxsplit=1)[0]
    return "\n".join(
        line for line in body.splitlines() if not line.lstrip().startswith("#")
    )


def _diagnostic_expected_branch() -> str:
    source = GUEST_DIAGNOSTIC.read_text(encoding="utf-8")
    match = re.search(r'^EXPECTED_KERNEL_BRANCH = "([^"]+)"', source, re.MULTILINE)
    assert match, f"EXPECTED_KERNEL_BRANCH not found in {GUEST_DIAGNOSTIC}"
    return match.group(1)


def test_every_arch_builds_the_same_kernel_branch() -> None:
    branches = _configured_kernel_branches()

    assert len(branches) == 1, (
        f"architectures pin different kernel branches {sorted(branches)}; "
        "one guest diagnostic cannot assert a branch that varies by arch"
    )


def test_guest_diagnostic_expects_the_configured_kernel_branch() -> None:
    configured = _configured_kernel_branches().pop()

    assert _diagnostic_expected_branch() == configured, (
        f"{BUILD_CONFIG.name} builds kernel branch {configured} but the guest "
        f"diagnostic expects {_diagnostic_expected_branch()}; a built image "
        "would fail its own diagnostics"
    )


def test_diagnostic_asserts_a_branch_not_an_open_ended_floor() -> None:
    code = _kernel_assertion_code()

    assert "EXPECTED_KERNEL_BRANCH" in code, (
        "the kernel diagnostic must compare against the configured branch, "
        "which is what this contract keeps in sync with the build pin"
    )
    assert "major >= " not in code, (
        "an open-ended major-version floor accepts kernels the guest was never "
        "built against and rejects the pinned branch whenever the pin moves "
        "backwards; assert the configured branch instead"
    )
