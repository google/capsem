"""A script's declared interpreter is its startup, with exact image arguments."""

from pathlib import Path
from types import SimpleNamespace

import pytest

from tests.qualification import image_command
from tests.qualification.image_command import command_matches

AGY_HEADER = '#!/bin/sh\nexec /usr/local/bin/agy-real --dangerously-skip-permissions "$@"\n'


@pytest.mark.parametrize("arguments", [[], ["--model", "a b", "-p", "hello"]])
def test_captured_agy_exec_preserves_every_original_argument(arguments):
    assert command_matches(
        ["agy", *arguments],
        "/usr/local/bin/agy",
        AGY_HEADER,
        ["/usr/local/bin/agy-real", "--dangerously-skip-permissions", *arguments],
    )


@pytest.mark.parametrize(
    "actual",
    [
        ["/usr/local/bin/agy-real"],
        ["/usr/local/bin/agy-real", "--skip-permissions"],
        ["/other/agy-real", "--dangerously-skip-permissions"],
        ["agy-real", "--dangerously-skip-permissions"],
        ["/usr/local/bin/agy-real", "--dangerously-skip-permissions", "extra"],
        ["/bin/sh", "/usr/local/bin/agy"],
    ],
)
def test_agy_exec_rejects_wrong_or_unfinished_execution(actual):
    assert not command_matches(["agy"], "/usr/local/bin/agy", AGY_HEADER, actual)


@pytest.mark.parametrize(
    "command,resolved,header",
    [
        (["other"], "/usr/local/bin/agy", AGY_HEADER),
        (["/usr/local/bin/agy"], "/usr/local/bin/agy", AGY_HEADER),
        (["agy"], "/other/agy", AGY_HEADER),
        (["agy"], "/usr/local/bin/agy", AGY_HEADER.rstrip("\n")),
        (["agy"], "/usr/local/bin/agy", AGY_HEADER + "# extra\n"),
        (["agy"], "/usr/local/bin/agy", AGY_HEADER.replace("/bin/sh", "/bin/bash")),
        (["agy"], "/usr/local/bin/agy", AGY_HEADER.replace("exec ", "exec  ")),
    ],
)
def test_agy_exec_is_only_the_exact_shipped_declaration(command, resolved, header):
    assert not command_matches(
        command, resolved, header, ["/usr/local/bin/agy-real", "--dangerously-skip-permissions"]
    )


def test_agy_exec_does_not_drop_or_reorder_original_arguments():
    for arguments in (["--model"], ["a b", "--model"], ["--model", "a b", "extra"]):
        assert not command_matches(
            ["agy", "--model", "a b"],
            "/usr/local/bin/agy",
            AGY_HEADER,
            ["/usr/local/bin/agy-real", "--dangerously-skip-permissions", *arguments],
        )


def test_agy_header_matches_the_actual_image_producer():
    dockerfile = Path(__file__).parents[2] / "images/agy/Dockerfile"
    escaped = AGY_HEADER.replace("\n", "\\n")
    assert f"printf '{escaped}' > /usr/local/bin/agy;" in dockerfile.read_text()


def test_captured_agy_pid1_uses_the_existing_bounded_header_read(monkeypatch):
    calls = []
    outputs = [
        "/usr/local/bin/agy-real\n--dangerously-skip-permissions\n",
        "/usr/local/bin/agy\n",
        AGY_HEADER,
    ]

    def execute(client, vm_id, command):
        assert client == "client" and vm_id == "owned-vm"
        calls.append(command)
        return {"exit_code": 0, "stdout_text": outputs[len(calls) - 1]}

    monkeypatch.setattr(image_command, "workload_exec", execute)
    image_command.assert_image_command("client", "owned-vm", SimpleNamespace(command=["agy"]))
    assert calls == [
        "tr '\\0' '\\n' < /proc/1/cmdline",
        "command -v agy",
        "head -c 256 /usr/local/bin/agy",
    ]


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
