"""Real prepared CLI check; finite manifest fetch, never native installation."""
import copy
import json
import os
import time
import tomllib
from contextlib import contextmanager
from functools import partial
from pathlib import Path

import tomli_w
from capsem_builder.crypto import sha256
from helpers.benchmark_assets import clone_benchmark_assets
from helpers.constants import ASSETS_VARIABLE, content_assets_root, host_bin_root
from helpers.service import ServiceInstance
from helpers.update_release import (
    _serve_manifest_release,
    _update_release_graph,
    _write_manifest_metadata,
    prepared_cli,
)
from psutil import Process

from .route_credentials import require_credential_metrics
from .route_requests import service_target
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count


def update_check_config():
    from capsem_builder.gate.benchmarkupdateschema import UpdateCheckConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return UpdateCheckConfig.model_validate(raw['benchmark']['routes']['update_check']).model_dump()


def bounded_json(path,maximum):
    assert path.is_file() and not path.is_symlink(), 'update graph/cache is not an original regular file'
    with path.open('rb') as source:
        body = source.read(maximum+1)
    assert 0 < len(body) <= maximum, 'update graph/cache exceeds configured bound'
    return json.loads(body)


def require_update_inputs(cfg=None):
    """Fail before any fixture: no shared Cargo target or synthetic graph fallback."""
    cfg = update_check_config() if cfg is None else cfg
    cli = prepared_cli(host_bin_root()/'capsem')
    if not os.environ.get(ASSETS_VARIABLE):
        raise RuntimeError('update check requires prepared manifest from CAPSEM_ASSETS_DIR')
    root = content_assets_root()
    graph = bounded_json(root/'manifest.json',cfg['max_manifest_bytes'])
    assert isinstance(graph.get('packages'),list) and graph['packages'], 'prepared release graph has no packages'
    assert isinstance(graph.get('runtime'),dict) and graph['runtime'].get('revision'), 'prepared release graph has no runtime'
    assert any(row.get('status','current') == 'current' for row in graph['packages']), 'prepared release graph has no current package'
    assert graph['runtime']['revision'] != cfg['runtime_revision'], 'candidate runtime is already installed'
    assert all(row.get('version') != cfg['binary_version'] for row in graph['packages']
        if row.get('status','current') == 'current'), 'candidate binary is already current'
    return cli,root,graph


@contextmanager
def update_target(label,cfg,inputs):
    cli,assets,graph = inputs
    candidate = _update_release_graph(copy.deepcopy(graph),cfg['binary_version'],cfg['runtime_revision'],
        min_binary=None,binary_files=None,asset_base=None)
    body = json.dumps(candidate,sort_keys=True,separators=(',',':')).encode()
    assert len(body) <= cfg['max_manifest_bytes']
    with _serve_manifest_release({cfg['manifest_path']:body}) as (base,requests):
        service = ServiceInstance(isolated_corp_config=True,update_cli=cli)
        try:
            service.assets_dir = clone_benchmark_assets(assets,service.home_dir/'assets')
            # Clone links remain readonly inputs; replace only our own manifest.
            path = service.assets_dir/'manifest.json'
            path.unlink()
            path.write_text(json.dumps(graph))
            _write_manifest_metadata(service.home_dir,base+cfg['manifest_path'])
            (service.home_dir/'settings.toml').write_text(tomli_w.dumps({
                'settings':{'app.auto_update':{'value':False,'modified':'update-check-fixture'}},
                'images':{'catalog':False}}))
            service.start()
            yield service_target(service,label),service,requests,body,base+cfg['manifest_path']
        finally:
            service.stop()
        assert requests == [cfg['manifest_path']], 'extra automatic/replayed manifest request'


def sample_update_check(cycle,requests,body,url):
    cfg,deadline = cycle.cfg,time.monotonic()+cycle.cfg['timeout_seconds']
    path = cycle.service.home_dir/'assets' / 'manifest-metadata.json'
    baseline = bounded_json(path,cfg['max_manifest_bytes'])
    assert 'checked_url' not in baseline and 'checked_at' not in baseline
    assert requests == [], 'automatic polling raced the original check'
    def verified(value):
        assert value['status'] == 'succeeded' and value['exit_code'] == 0, 'real CLI update check failed'
        assert value['command'] == {'program':str(cycle.service.update_cli),'args':['update','--check']}
        assert isinstance(value['stdout'],str) and value['stdout'].strip()
        assert isinstance(value['stderr'],str)
        assert len((value['stdout']+value['stderr']).encode()) <= cfg['max_cli_output_bytes']
        assert 'Development build detected' not in value['stdout'], 'Development no-op is not a check'
        assert requests == [cfg['manifest_path']], 'original check did not fetch exactly one manifest'
        cache = bounded_json(path,cfg['max_manifest_bytes'])
        expected_hash = sha256(body).hexdigest()
        assert cache['checked_url'] == url and cache['channel_hash'] == expected_hash
        assert cache['manifest_url'] == baseline['manifest_url'] == url
        assert cache['validation_status'] == 'valid' and cache['validation_error'] is None
        assert type(cache['checked_at']) is int and cache['checked_at'] > 0
        assert cache['latest_version'] == cfg['binary_version'] and cache['update_available'] is True
        assert cache['latest_assets'] == cache['latest_images'] == cfg['runtime_revision']
        assert cache['assets_update_available'] is True and cache['images_update_available'] is True
        status = cycle.read('/update/status',deadline=deadline)
        assert status['channel_url'] == url and status['channel_hash'] == expected_hash
        assert status['checked_at'] == cache['checked_at'] and status['validation_status'] == 'valid'
        # UpdateStatusResponse intentionally omits absent Option fields.
        assert status.get('validation_error') is None and status.get('last_error') is None and status['stale'] is False
        assert status['binary']['latest'] == cfg['binary_version'] and status['binary']['update_available'] is True
        assert status['assets']['latest'] == status['images']['latest'] == cfg['runtime_revision']
        assert status['assets']['update_available'] is True and status['images']['update_available'] is True
        assert requests == [cfg['manifest_path']], 'status read or poll added an upstream request'
    cycle.observe_control('POST /update/check','real-cli-manifest-fetch',
        lambda:cycle.call('POST','/update/check',{},deadline=deadline),verified,deadline=deadline)


def collect_update_check(quick):
    cfg,metrics = update_check_config(),{}
    inputs = require_update_inputs(cfg)
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with update_target(label,cfg,inputs) as (target,service,requests,body,url):
                transport = target.transport_factory()
                try:
                    assert service.proc is not None
                    cycle = Cycle(transport,target,service,cfg,metrics)
                    measure(metrics,f'{label}.update_check.{index}',Process(service.proc.pid),
                        partial(sample_update_check,cycle,requests,body,url))
                finally:
                    transport.close()
    require_credential_metrics(metrics,cfg,quick,resource_domain='update_check')
    return metrics
