"""Resolving a channel's source manifest uses the release's own GitHub login.

`release-binaries` dispatches its hosted lane through `gh`, so the operator's
`gh` login is already a release prerequisite. The source-manifest fetch alone
also demanded `GITHUB_TOKEN` in the environment, and found out it was missing
eight minutes in, after the release's citadel, contract and build-system
suites had all passed.
"""

from __future__ import annotations

import subprocess

import pytest
from capsem_builder.release.tools import fetch_channel_source_manifest as fetch


def test_an_exported_token_is_used_as_is(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_TOKEN", "exported")
    monkeypatch.setattr(fetch.subprocess, "run", pytest.fail)
    assert fetch.github_token() == "exported"


def test_without_one_the_gh_login_supplies_it(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)
    calls: list[list[str]] = []

    def run(command, **kwargs):
        calls.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="from-gh\n", stderr="")

    monkeypatch.setattr(fetch.subprocess, "run", run)
    assert fetch.github_token() == "from-gh"
    assert calls == [["gh", "auth", "token"]]


@pytest.mark.parametrize(
    "outcome",
    [
        subprocess.CompletedProcess(["gh"], 1, stdout="", stderr="not logged in"),
        subprocess.CompletedProcess(["gh"], 0, stdout="\n", stderr=""),
        FileNotFoundError("gh"),
    ],
)
def test_no_token_anywhere_is_none(monkeypatch: pytest.MonkeyPatch, outcome) -> None:
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)

    def run(command, **kwargs):
        if isinstance(outcome, BaseException):
            raise outcome
        return outcome

    monkeypatch.setattr(fetch.subprocess, "run", run)
    assert fetch.github_token() is None
