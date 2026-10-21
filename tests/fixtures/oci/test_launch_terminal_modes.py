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
