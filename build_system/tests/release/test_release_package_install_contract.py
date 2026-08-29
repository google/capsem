"""What the Debian package's post-install may assume about the machine.

`verify-release-candidate` installs the candidate `.deb` inside a plain
`ubuntu:24.04` container. That job was skipped in all twenty-two binary
release attempts, so this path had never run once. It fails: the postinst
guards service registration with `command -v systemctl`, and in a container
the binary is present -- the desktop dependencies pull systemd in -- while
systemd is not the init system. `systemctl --user` then has no manager to
talk to, registration fails, and dpkg leaves the package unconfigured.

The script already treats "no systemd" as a normal outcome: there is no else
branch, it simply proceeds to `complete`. Only the detection was wrong.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]
POSTINST = PROJECT_ROOT / "build_system" / "packaging" / "linux" / "deb-postinst.sh"

# systemd's own documented answer to "am I the init system" -- `sd_booted(3)`
# is specified as the existence of this directory.
BOOTED_MARKER = "/run/systemd/system"


def _register_service_guard() -> str:
    """The `if` that decides whether the package registers a user service."""
    text = POSTINST.read_text(encoding="utf-8")
    phase = text.index('CAPSEM_INSTALL_PHASE="register_service"')
    guard = re.search(r"^if (.+); then$", text[phase:], re.MULTILINE)
    assert guard, "the register_service phase no longer opens with an `if`"
    return guard.group(1)


def test_registration_requires_a_booted_systemd_not_just_the_binary() -> None:
    guard = _register_service_guard()
    assert BOOTED_MARKER in guard, (
        "the postinst decides whether to register a systemd user service by "
        f"looking for the systemctl binary, but not for {BOOTED_MARKER}. A "
        "container has the first without the second, so the package installs "
        "the binary, tries to reach a service manager that is not running, "
        f"and dpkg fails. guard was: if {guard}; then"
    )


def test_a_missing_service_manager_is_not_an_install_failure() -> None:
    """Skipping registration must reach `complete`, not `exit 1`."""
    text = POSTINST.read_text(encoding="utf-8")
    phase = text.index('CAPSEM_INSTALL_PHASE="register_service"')
    block = text[phase : text.index("\nfi\n", phase)]
    if "\nelse\n" in block:
        skipped = block[block.index("\nelse\n") :]
        assert "exit " not in skipped, (
            "a machine with no service manager must still finish installing; "
            f"the skip path exits:\n{skipped}"
        )


def test_the_skip_is_recorded_rather_than_silent() -> None:
    """Every other decision in this script emits an event; so must this one."""
    text = POSTINST.read_text(encoding="utf-8")
    assert "event=service_registration_skipped" in text, (
        "the package can decline to register a service and say nothing, which "
        "is indistinguishable in a log from having registered one"
    )
