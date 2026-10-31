"""Terminal acceptance must observe the complete event it is asserting."""

import pytest

from tests.ironbank.kingslanding.test_terminal_state import terminal_summary


def test_notification_readiness_waits_past_an_already_retained_title():
    states = [
        {"title": "Retained", "bell_count": 0},
        {"title": "Retained", "bell_count": 0, "last_notification": None},
        {"title": "Retained", "bell_count": 0,
         "last_notification": {"title": "Earlier", "body": "Ready"}},
        {"title": "Retained", "bell_count": 0,
         "last_notification": {"title": "Recovered", "body": "Pending"}},
        {"title": "Retained", "bell_count": 0,
         "last_notification": {"title": "Recovered", "body": "Ready"}},
    ]

    class Client:
        def get(self, path):
            assert path == "/vms/list"
            return {"sandboxes": [{"id": "vm", "terminal": states.pop(0)}]}

    summary = terminal_summary(Client(), "vm", "Retained", notification=("Recovered", "Ready"))
    assert summary["last_notification"] == {"title": "Recovered", "body": "Ready"}
    assert not states


def test_bell_readiness_waits_after_the_title_arrives():
    states = [
        {"title": "Ready", "bell_count": 0},
        {"title": "Ready", "bell_count": 1},
    ]

    class Client:
        def get(self, path):
            assert path == "/vms/list"
            return {"sandboxes": [{"id": "vm", "terminal": states.pop(0)}]}

    assert terminal_summary(Client(), "vm", "Ready", bell_count=1)["bell_count"] == 1
    assert not states


def test_notification_readiness_cannot_accept_a_never_delivered_event():
    class Client:
        def get(self, _path):
            return {"sandboxes": [{"id": "vm", "terminal": {"title": "Retained"}}]}

    with pytest.raises(AssertionError, match="Recovered"):
        terminal_summary(Client(), "vm", "Retained", notification=("Recovered", "Ready"), timeout=0.01)
