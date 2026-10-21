"""The workload owns cooked/raw output processing; its relay preserves bytes."""

import os
import tty

from tests.fixtures.oci.test_launch_config import launcher

__all__ = ["launcher"]


def test_initial_cooked_output_can_be_switched_to_raw_by_the_application(launcher):
    master, terminal = launcher.console_terminal()
    try:
        os.write(terminal, b"shell output\n")
        assert os.read(master, 128) == b"shell output\r\n"
        tty.setraw(terminal)
        os.write(terminal, b"abcdefgh\n\n\b\bX")
        assert os.read(master, 128) == b"abcdefgh\n\n\b\bX"
    finally:
        os.close(master)
        os.close(terminal)


def test_interactive_workload_advertises_the_actual_terminal_color_capabilities(launcher):
    from tests.fixtures.oci.test_launch_config import SECURITY, image, unpacked

    source = unpacked()
    source["process"]["env"] += ["TERM=xterm", "COLORTERM="]
    options = {**SECURITY, "args": [], "env": {}}
    configured = launcher.configure(source, image(), options, detached=True)
    env = dict(entry.split("=", 1) for entry in configured["process"]["env"])
    assert env["TERM"] == "xterm-256color"
    assert env["COLORTERM"] == "truecolor"
    configured = launcher.configure(source, image(), {**options, "env": {"TERM": "dumb", "COLORTERM": ""}}, detached=True)
    env = dict(entry.split("=", 1) for entry in configured["process"]["env"])
    assert (env["TERM"], env["COLORTERM"]) == ("dumb", "")
