"""Release channel root-page contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    test_channel_descriptions,
    test_channel_list_has_no_status_or_records_theater,
    test_channel_manifest_revision_not_selected_manifest,
    test_channel_name_not_repeated,
    test_root_channel_manifest_metadata,
)


def test_root_channel_descriptions_not_duplicate_ids() -> None:
    test_channel_name_not_repeated()
    test_channel_descriptions()


def test_root_channel_table_uses_manifest_version_last_updated_labels() -> None:
    test_channel_manifest_revision_not_selected_manifest()
    test_channel_list_has_no_status_or_records_theater()
    test_root_channel_manifest_metadata()
