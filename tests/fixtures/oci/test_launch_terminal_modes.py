"""The workload owns cooked/raw output processing; its relay preserves bytes."""

from tests.fixtures.oci.test_launch_config import launcher

__all__ = ["launcher"]


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
