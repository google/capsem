"""A script's declared interpreter is its startup, with exact image arguments."""

import pytest

from tests.qualification.image_command import command_matches


@pytest.mark.parametrize(
    "command,resolved,header,actual",
    [
        (["sleep", "infinity"], "/usr/bin/sleep", "", ["sleep", "infinity"]),
        (["sleep", "infinity"], "/usr/bin/sleep", "", ["/usr/bin/sleep", "infinity"]),
        (
            ["codex"],
            "/usr/local/bin/codex",
            "#!/usr/bin/env node\n",
            ["node", "/usr/local/bin/codex"],
        ),
        (
            ["script", "a b"],
            "/app/script",
            "#!/usr/bin/python3 -O\n",
            ["/usr/bin/python3", "-O", "/app/script", "a b"],
        ),
        (
            ["script", "--run"],
            "/app/script",
            "#!/usr/bin/env -S python3 -O\n",
            ["python3", "-O", "/app/script", "--run"],
        ),
    ],
)
def test_the_command_or_its_own_interpreter_runs(command, resolved, header, actual):
    assert command_matches(command, resolved, header, actual)


@pytest.mark.parametrize(
    "actual",
    [
        ["bash"],
        ["node", "/other/codex"],
        ["python3", "/usr/local/bin/codex"],
        ["node", "/usr/local/bin/codex", "extra"],
        ["codex", "extra"],
    ],
)
def test_wrong_program_script_interpreter_or_arguments_do_not_qualify(actual):
    assert not command_matches(["codex"], "/usr/local/bin/codex", "#!/usr/bin/env node\n", actual)


def test_an_env_option_or_an_empty_command_is_not_a_startup_substitute():
    assert not command_matches(
        ["script"],
        "/app/script",
        "#!/usr/bin/env -i python3\n",
        ["python3", "/app/script"],
    )
    assert not command_matches([], "", "", ["bash"])


@pytest.mark.parametrize(
    "command", [["/usr/local/bin/capsem-gui"], ["capsem-gui", "a b", "--example"]]
)
def test_shared_gui_launcher_delegates_exact_command_and_arguments_to_private_bus(command):
    assert command_matches(
        command,
        "/usr/local/bin/capsem-gui",
        "#!/bin/sh\n",
        ["dbus-run-session", "--", "/usr/local/bin/capsem-gui", *command[1:]],
    )


@pytest.mark.parametrize(
    "actual",
    [
        ["dbus-run-session", "--", "/other/capsem-gui"],
        ["dbus-run-session", "/usr/local/bin/capsem-gui"],
        ["dbus-run-session", "--", "/usr/local/bin/capsem-gui", "extra"],
        ["env", "dbus-run-session", "--", "/usr/local/bin/capsem-gui"],
        ["dbus-run-session", "--", "dbus-run-session", "--", "/usr/local/bin/capsem-gui"],
        ["/other/dbus-run-session", "--", "/usr/local/bin/capsem-gui"],
    ],
)
def test_gui_delegation_does_not_strip_arbitrary_prefixes_or_change_arguments(actual):
    assert not command_matches(
        ["/usr/local/bin/capsem-gui"], "/usr/local/bin/capsem-gui", "#!/bin/sh\n", actual
    )


@pytest.mark.parametrize(
    "command,resolved",
    [
        (["codex"], "/usr/local/bin/codex"),
        (["/other/capsem-gui"], "/other/capsem-gui"),
        (["/other/launcher"], "/usr/local/bin/capsem-gui"),
    ],
)
def test_dbus_is_not_a_generic_startup_wrapper(command, resolved):
    assert not command_matches(
        command, resolved, "#!/bin/sh\n", ["dbus-run-session", "--", resolved]
    )


def test_gui_delegation_preserves_missing_and_changed_configured_arguments():
    command = ["capsem-gui", "a b"]
    for arguments in ([], ["a", "b"], ["different"]):
        assert not command_matches(
            command,
            "/usr/local/bin/capsem-gui",
            "#!/bin/sh\n",
            ["dbus-run-session", "--", "/usr/local/bin/capsem-gui", *arguments],
        )
