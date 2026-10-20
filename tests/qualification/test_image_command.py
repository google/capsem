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
    assert not command_matches(
        ["codex"], "/usr/local/bin/codex", "#!/usr/bin/env node\n", actual
    )


def test_an_env_option_or_an_empty_command_is_not_a_startup_substitute():
    assert not command_matches(
        ["script"],
        "/app/script",
        "#!/usr/bin/env -i python3\n",
        ["python3", "/app/script"],
    )
    assert not command_matches([], "", "", ["bash"])
