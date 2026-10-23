"""Existing isolated service fixture, with no VM or image producer."""
from contextlib import contextmanager

import tomli_w
from helpers.benchmark_assets import clone_benchmark_assets
from helpers.constants import ASSETS_DIR
from helpers.service import ServiceInstance

from .route_requests import service_target


@contextmanager
def network_target(label):
    service = ServiceInstance(isolated_corp_config=True)
    try:
        service.assets_dir = clone_benchmark_assets(ASSETS_DIR, service.home_dir / 'benchmark-assets')
        (service.home_dir / 'settings.toml').write_text(tomli_w.dumps({
            'settings': {'app.auto_update': {'value': False, 'modified': 'route-fixture'}},
            'images': {'catalog': False},
        }))
        service.start()
        yield service_target(service, label), service
    finally:
        # On an uncertain failed acknowledgement do not replay a mutation;
        # the private home/service resource owner performs ordinary teardown.
        service.stop()
