"""The guest kernel assertion must track the exact source selected for the image.

`config/docker/image/build.toml` decides which exact kernel source Capsem builds,
and a guest diagnostic asserts the booted kernel is that version. Those two
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


def _configured_kernel_version() -> str:
    return tomllib.loads(BUILD_CONFIG.read_text(encoding="utf-8"))["build"][
        "kernel"
    ]["version"]


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


def _diagnostic_expected_version() -> str:
    source = GUEST_DIAGNOSTIC.read_text(encoding="utf-8")
    match = re.search(r'^EXPECTED_KERNEL_VERSION = "([^"]+)"', source, re.MULTILINE)
    assert match, f"EXPECTED_KERNEL_VERSION not found in {GUEST_DIAGNOSTIC}"
    return match.group(1)


def test_kernel_version_is_one_common_exact_source() -> None:
    assert re.fullmatch(r"\d+\.\d+\.\d+", _configured_kernel_version())


def test_guest_diagnostic_expects_the_configured_kernel_version() -> None:
    configured = _configured_kernel_version()

    assert _diagnostic_expected_version() == configured, (
        f"{BUILD_CONFIG.name} builds kernel {configured} but the guest "
        f"diagnostic expects {_diagnostic_expected_version()}; a built image "
        "would fail its own diagnostics"
    )


def test_diagnostic_asserts_the_exact_version_not_a_branch_or_floor() -> None:
    code = _kernel_assertion_code()

    assert "EXPECTED_KERNEL_VERSION" in code, (
        "the kernel diagnostic must compare against the configured exact version, "
        "which is what this contract keeps in sync with the build source"
    )
    assert 'split(".")' not in code, "the diagnostic must not weaken the pin to a branch"
    assert "major >= " not in code, (
        "an open-ended major-version floor accepts kernels the guest was never "
        "built against and rejects the exact pin whenever it moves backwards; "
        "assert the configured version instead"
    )
