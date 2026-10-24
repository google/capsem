"""Existing pinned local image and isolated service owners; no image preparation."""
from __future__ import annotations

import contextlib
import tempfile
from pathlib import Path

from helpers.service import ServiceInstance

from .route_requests import service_target


def fixture_inputs():
    # Keep real OCI fixture imports at launch, like the owning collector: pure
    # source/unit imports must not materialize product/image dependencies.
    from tests.fixtures.oci.pinned_image import reference_image
    from tests.fixtures.oci.published_root import ready
    from tests.fixtures.oci.registry import grant_exact, layout_registry
    return reference_image, ready, grant_exact, layout_registry


@contextlib.contextmanager
def lifecycle_target(label, *, prepare=None):
    reference_image, ready, grant_exact, layout_registry = fixture_inputs()
    layout = reference_image.ready()
    ready(layout)  # Existing immutable producer receipt is mandatory; never build/fetch.
    with (
        tempfile.TemporaryDirectory(prefix='capsem-lifecycle-routes-') as temporary,
        layout_registry(Path(temporary), layout, reference_image.settings().name) as (reference, certificate, _),
    ):
        service = ServiceInstance(isolated_corp_config=True)
        try:
            catalog = reference.split('/', 1)[0] + '/library/catalog:stable'
            grant_exact(service.home_dir, reference, catalog=catalog, certificate=certificate)
            if prepare is not None:
                prepare(service)
            service.start()
            yield service_target(service, label), service, reference, certificate
        finally:
            service.stop()
