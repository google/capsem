"""Only isolated service files and the canonical unapproved local tool cache."""
from __future__ import annotations

from contextlib import contextmanager

import tomli_w
from helpers.benchmark_assets import clone_benchmark_assets
from helpers.constants import ASSETS_DIR
from helpers.service import ServiceInstance

from .route_fixtures import corpus_config, seed_metadata
from .route_policy import call, validate_baseline
from .route_requests import service_target


@contextmanager
def policy_target(label, cfg):
    service = ServiceInstance(isolated_corp_config=True)
    try:
        service.assets_dir = clone_benchmark_assets(ASSETS_DIR, service.home_dir / 'benchmark-assets')
        (service.home_dir / 'settings.toml').write_text(tomli_w.dumps({
            'settings': {cfg['setting_id']: {'value': cfg['setting_baseline'], 'modified': 'route-fixture'},
                         'app.auto_update': {'value': False, 'modified': 'route-fixture'}},
            'images': {'catalog': False},
            'plugins': {cfg['plugin_id']: {'mode': cfg['plugin_baseline'], 'detection_level': 'critical'}},
            'default': {'mcp': {'action': cfg['default_baseline']}},
        }))
        assert not (service.home_dir / 'corp.toml').exists()
        corpus = corpus_config()
        assert corpus['mcp_tools'] == [cfg['tool_id']], 'policy case needs the canonical local cache corpus'
        seed_metadata(service, corpus)
        service.start()
        target = service_target(service, label)
        transport = target.transport_factory()
        try:
            # Canonical route creates its own managed rule. This setup edit is
            # untimed; the measured edit must change the already-existing rule.
            _, initialized = call(transport, target, 'PATCH', f'/mcp/servers/{cfg["server_id"]}/tools/{cfg["tool_id"]}/edit', cfg, {'action': cfg['tool_baseline']})
            assert initialized['action'] == cfg['tool_baseline']
            validate_baseline(transport, target, service.home_dir, cfg)
        finally:
            transport.close()
        yield target, service
    finally:
        service.stop()
