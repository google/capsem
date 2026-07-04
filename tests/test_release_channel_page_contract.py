"""Release channel root-page contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    test_channel_descriptions,
    test_channel_name_not_repeated,
)


def test_root_channel_descriptions_not_duplicate_ids() -> None:
    test_channel_name_not_repeated()
    test_channel_descriptions()
