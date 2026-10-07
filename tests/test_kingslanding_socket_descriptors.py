"""The kingslanding switch helper counts a live process's sockets while it runs.

`socket_descriptors` lists `/proc/<pid>/fd` and then reads each link. The
switch opens and closes sockets throughout, so a descriptor can close between
the two calls; a stable co-work profile qualification failed
`test_network_switch.py` on exactly that `FileNotFoundError`. A descriptor that
is gone is not held, so it is skipped, and anything else still fails loudly.
"""

import sys

import pytest

from tests.ironbank.kingslanding import network

pytestmark = pytest.mark.skipif(sys.platform != "linux", reason="reads /proc")


def _fake_fd_table(monkeypatch, links, failing):
    monkeypatch.setattr(network.os, "listdir", lambda root: sorted(links) + sorted(failing))

    def readlink(path):
        fd = path.rsplit("/", 1)[1]
        if fd in failing:
            raise failing[fd]
        return links[fd]

    monkeypatch.setattr(network.os, "readlink", readlink)


def test_a_descriptor_closed_mid_listing_is_not_counted(monkeypatch) -> None:
    _fake_fd_table(
        monkeypatch,
        {"3": "socket:[11]", "4": "pipe:[12]", "5": "socket:[13]"},
        {"12": FileNotFoundError(2, "No such file or directory")},
    )

    count, listing = network.socket_descriptors(4242)

    assert count == 2
    assert '"12"' not in listing


def test_an_unreadable_descriptor_still_fails(monkeypatch) -> None:
    _fake_fd_table(monkeypatch, {"3": "socket:[11]"}, {"7": PermissionError(13, "Permission denied")})

    with pytest.raises(PermissionError):
        network.socket_descriptors(4242)
