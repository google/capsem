"""An image the policy does not grant never runs, and its registry is never asked.

The service checks an image's source against `[images] sources` before any
registry access, and admits it on its resolved digest against
`[images] admit`. A test registry is a fresh localhost port; every other
kingslanding test grants it first (`grant_image`). This one does not.
"""

import subprocess

import pytest
from helpers.constants import CODE_PROFILE_ID

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import cli, environment, grant_image, service

__all__ = ["service"]

pytestmark = pytest.mark.integration


def run_image(service, reference, certificate):
    return subprocess.run(
        [
            *cli(
                service,
                "run",
                "--profile",
                CODE_PROFILE_ID,
                "--registry-ca",
                str(certificate),
            ),
            "--image",
            reference,
            "true",
        ],
        env=environment(service),
        capture_output=True,
        timeout=180,
        check=False,
    )


def test_an_ungranted_image_is_refused_before_its_registry_is_contacted(
    service, tmp_path
):
    settings = service.home_dir / "settings.toml"
    before = settings.read_text() if settings.exists() else None
    try:
        # No grants, and no catalog: an ungranted source would otherwise send
        # the service to ghcr.io for the official catalog, which this
        # hermetic suite never contacts (test_image_catalog serves its own).
        settings.write_text("[images]\ncatalog = false\n")
        with registry(tmp_path) as (reference, certificate, requests):
            result = run_image(service, reference, certificate)
            refused = result.stdout.decode(errors="replace") + result.stderr.decode(
                errors="replace"
            )
            assert result.returncode != 0, refused
            assert "not allowed" in refused, refused
            assert requests == [], f"the refused registry was contacted: {requests}"

            # Granted, the same image runs.
            grant_image(service, reference)
            granted = run_image(service, reference, certificate)
            assert granted.returncode == 0, granted.stderr.decode(errors="replace")
    finally:
        if before is None:
            settings.unlink(missing_ok=True)
        else:
            settings.write_text(before)
