"""Compatibility release-site generation contract gates."""

from test_release_site_generated_from_json import test_no_profile_catalog_side_channel


def test_release_site_forbids_profile_catalog_side_channel() -> None:
    test_no_profile_catalog_side_channel()
