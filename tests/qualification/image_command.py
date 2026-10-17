"""Compare PID1 with the command and interpreter the candidate image declares."""

import shlex

from helpers.image_session import workload_exec


def command_matches(command, resolved, header, actual):
    if not command:
        return False
    if actual in (command, [resolved, *command[1:]]):
        return True
    first_line = header.splitlines()[0] if header else ""
    if not first_line.startswith("#!"):
        return False
    words = first_line[2:].strip().split(maxsplit=1)
    if not words:
        return False
    interpreter, optional = words[0], words[1] if len(words) == 2 else ""
    if interpreter.rsplit("/", 1)[-1] == "env":
        if optional.startswith("-S "):
            try:
                prefix = shlex.split(optional[3:])
            except ValueError:
                return False
        elif optional and not optional.startswith("-"):
            prefix = [optional]
        else:
            return False
        if not prefix or prefix[0].startswith("-"):
            return False
    else:
        prefix = [interpreter, *([optional] if optional else [])]
    return actual == [*prefix, resolved, *command[1:]]


def assert_image_command(client, vm_id, candidate):
    result = workload_exec(client, vm_id, "tr '\\0' '\\n' < /proc/1/cmdline")
    assert result.get("exit_code") == 0, result
    actual = result["stdout_text"].splitlines()
    if candidate.command and actual == candidate.command:
        return
    assert candidate.command, "the candidate must declare its own startup command"
    resolved = workload_exec(
        client, vm_id, f"command -v {shlex.quote(candidate.command[0])}"
    )
    assert resolved.get("exit_code") == 0, resolved
    path = resolved["stdout_text"].strip()
    assert path and "\n" not in path, resolved
    if command_matches(candidate.command, path, "", actual):
        return
    # Bound the read to the kernel's script-header limit; never emit a binary
    # or entire CLI launcher just to identify its interpreter.
    header = workload_exec(client, vm_id, f"head -c 256 {shlex.quote(path)}")
    assert header.get("exit_code") == 0, header
    assert command_matches(candidate.command, path, header["stdout_text"], actual), (
        candidate.command,
        path,
        header["stdout_text"].splitlines()[:1],
        actual,
    )
