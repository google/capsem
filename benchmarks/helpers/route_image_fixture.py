"""Existing pinned image/registry and service-home owners, outside timing."""
from __future__ import annotations

import contextlib
import json
import tempfile
from pathlib import Path

from helpers.benchmark_assets import clone_benchmark_assets
from helpers.constants import ASSETS_DIR
from helpers.service import ServiceInstance

from tests.fixtures.oci.image_build import prepare_local_evidence
from tests.fixtures.oci.layout_registry import layout_registry
from tests.fixtures.oci.pinned_image import reference_image

from .route_requests import service_target


@contextlib.contextmanager
def prepared_registry(cfg):
    # Existing image/cache preparation owns export/scan/repack. This is not a
    # download benchmark and never substitutes metadata from another repository.
    layout = reference_image.ready()
    # Match OCI producer staging: bounded TMPDIR may be outside Colima's mounts.
    with tempfile.TemporaryDirectory(prefix='capsem-image-routes-', dir=reference_image.stage()) as temporary:
        root = Path(temporary)
        (root / 'registry').mkdir()
        expected = {}
        def evidence(reference):
            output = prepare_local_evidence(reference_image, reference, root / 'evidence', inspect_timeout=cfg['inspect_timeout_seconds'])
            expected.update(json.loads((output / 'packages.json').read_bytes()))
            return output
        with layout_registry(root / 'registry', layout, 'dev', official=True, evidence=evidence) as (reference, certificate, requests):
            assert expected and expected['subject'] == reference
            yield reference, certificate, requests, expected


@contextlib.contextmanager
def image_target(label, reference, certificate):
    service = ServiceInstance()
    try:
        service.assets_dir = clone_benchmark_assets(ASSETS_DIR, service.home_dir / 'benchmark-assets')
        manifest_path = service.assets_dir / 'manifest.json'
        manifest = json.loads(manifest_path.read_bytes())
        manifest.pop('asset_base', None)
        manifest_path.unlink()
        manifest_path.write_text(json.dumps(manifest))
        metadata_path = service.assets_dir / 'manifest-metadata.json'
        metadata = json.loads(metadata_path.read_bytes())
        metadata['manifest_url'] = (ASSETS_DIR / 'manifest.json').resolve().as_uri()
        metadata_path.write_text(json.dumps(metadata))
        # Exactly the same catalogue-only policy as the canonical acceptance fixture.
        (service.home_dir / 'settings.toml').write_text('[images]\ncatalog = ' + json.dumps(reference.split('/')[0] + '/library/catalog:stable') + '\ncatalog_ca = ' + json.dumps(str(certificate)) + '\n')
        # ServiceInstance owns CAPSEM_HOME, so cold is a real independent cache.
        assert not (service.home_dir / 'cache').exists()
        service.start()
        target = service_target(service, label)
        yield target, service
    finally:
        service.stop()
