"""Finite catalogue/materialization receipts; no statistics or cold-cache fiction."""
from __future__ import annotations

import filecmp
import re
import time
import tomllib
from pathlib import Path
from urllib.parse import quote

from .route_measurements import Observations
from .route_requests import json_request
from .route_validation import SCHEMAS
from .stream_measurements import measure, sample_count

ROOT = Path(__file__).resolve().parents[2]


def image_config() -> dict:
    return tomllib.loads((ROOT / 'config/gate.toml').read_text())['benchmark']['routes']['images']


def validate_catalog(payload, reference: str, architecture: str) -> None:
    authority = reference.split('/')[0]
    assert payload['catalog']['reference'] == f'{authority}/library/catalog:stable'
    assert re.fullmatch(r'sha256:[0-9a-f]{64}', payload['catalog']['digest'])
    assert payload['catalog']['channel'] == 'stable'
    (row,) = payload['images']
    assert row['name'] == 'dev' and row['image'] == reference
    assert row['cached'] in {'unknown', 'missing', 'partial', 'ready'}
    app = row['app']
    assert app['image'] == reference and app['architecture'] == architecture
    evidence = {entry['kind']: entry for entry in app['evidence']}
    assert {'obom', 'package_inventory'} <= evidence.keys()
    for kind in ('obom', 'package_inventory'):
        assert evidence[kind]['reference'].startswith(reference.split('@')[0] + '@sha256:')
        assert evidence[kind]['media_type']


def validate_inventory(payload, expected: dict) -> None:
    assert expected['packages'] and all(row['name'] and row['version'] and row['architecture'] for row in expected['packages'])
    assert payload['image'] == expected['subject'] and payload['architecture'] == expected['architecture']
    assert payload['artifact_reference']
    # Producer evidence also binds binary paths/digests. The generated public
    # DTO projects its package/tool fields just as the service does.
    public = SCHEMAS['/images/{name}/inventory'].validate_python({
        'image': expected['subject'], 'architecture': expected['architecture'],
        'artifact_reference': payload['artifact_reference'], 'inventory': expected,
    }).model_dump(by_alias=True)
    assert payload['inventory'] == public['inventory']


def require_transfers(requests: list[str], *, cold: bool) -> None:
    blobs = [path for path in requests if path.startswith('/v2/library/dev/blobs/')]
    assert bool(blobs) == cold, f'cold={cold}, unexpected image blob transfers: {requests}'
    if cold:
        assert len(set(blobs)) >= 2, 'cold pull must fetch actual config and image/root data'


def remove_owned_asset(owned: Path, path: str) -> None:
    selected = Path(path)
    if '..' in selected.parts or not selected.is_relative_to(owned.absolute()) or not selected.parent.resolve().is_relative_to(owned.resolve()):
        raise ValueError('asset path is outside the owned clone')
    # Unlink the owned directory entry, never resolve its shared-cache target.
    selected.unlink()


def validate_assets(payload, *, downloaded: int | None = None) -> None:
    assert payload['ready'] is True and payload['downloading'] is False
    assert payload['errors'] == [] and payload.get('reconcile_error') is None
    if downloaded is not None:
        assert payload['downloaded'] == downloaded
    assert payload['assets']
    for asset in payload['assets']:
        path = Path(asset['path'])
        assert asset['status'] == 'present' and path.is_file()
        assert path.stat().st_size == asset['actual_size'] == asset['expected_size']
        assert re.fullmatch(r'(blake3:)?[0-9a-f]{64}', asset['expected_hash'])


def receipt(transport, target, method: str, path: str, cfg: dict, body=None):
    result, payload = json_request(transport, target, method, path, body,
        timeout=cfg['timeout_seconds'], maximum=cfg['max_response_bytes'])
    schema_path = path.split('?', 1)[0]
    if schema_path.endswith('/inventory'):
        schema_path = '/images/{name}/inventory'
    if schema_path in SCHEMAS:
        SCHEMAS[schema_path].validate_json(result.body)
    return result, payload


def settled(transport, target, cfg):
    deadline = time.monotonic() + cfg['timeout_seconds']
    while time.monotonic() < deadline:
        remaining = {**cfg, 'timeout_seconds': max(0.001, deadline - time.monotonic())}
        _, status = receipt(transport, target, 'GET', '/assets/status', remaining)
        if not status['downloading']:
            return status
        time.sleep(min(cfg['poll_seconds'], max(0, deadline - time.monotonic())))
    raise TimeoutError('asset materialization did not complete')


def sample_images(target, service, reference, certificate, requests, expected, cfg, quick, metrics):
    """One fresh service home is one cold-pull trial; mutation counts are finite."""
    collected: dict[str, Observations] = {}
    keys = {case: f'{operation} [{case}]' for operation, endpoint in cfg['endpoints'].items() for case in endpoint['cases']}
    transport = target.transport_factory()
    def observe(case, method, path, validate, body=None):
        result, payload = receipt(transport, target, method, path, cfg, body)
        validate(payload)
        collected.setdefault(case, Observations()).append(result)
        return payload
    def catalogue(payload):
        validate_catalog(payload, reference, expected['architecture'])
    try:
        # Unmeasured startup and first catalogue load. No image cache existed at start.
        validate_assets(settled(transport, target, cfg))
        _, listing = receipt(transport, target, 'GET', '/images', cfg)
        catalogue(listing)
        for _ in range(sample_count(cfg, 'cached_samples', quick)):
            before = len(requests)
            observe('catalog_cached', 'GET', '/images', catalogue)
            assert not any('/library/catalog/' in path for path in requests[before:]), 'cached catalogue unexpectedly refreshed'
        for _ in range(sample_count(cfg, 'refresh_samples', quick)):
            before = len(requests)
            observe('catalog_refresh', 'GET', '/images?refresh=true', catalogue)
            assert any(path == '/v2/library/catalog/manifests/stable' for path in requests[before:]), 'refresh did not read registry'
        pull = {'image': 'dev', 'registry': {'ca_pem': certificate.read_text()}}
        def pulled(payload):
            assert payload['image'] == 'dev' and payload['resolved'] == reference
            assert payload['digest'] == reference.split('@')[1]
        before = len(requests)
        observe('pull_cold', 'POST', '/images/pull', pulled, pull)
        require_transfers(requests[before:], cold=True)
        for _ in range(sample_count(cfg, 'warm_pulls', quick)):
            before = len(requests)
            observe('pull_warm', 'POST', '/images/pull', pulled, pull)
            require_transfers(requests[before:], cold=False)
        path = '/images/dev/inventory?image=' + quote(reference, safe='')
        # Initial artifact hydration is setup. Warm verified reads may still
        # authenticate and fetch manifests; this is not a registry-offline claim.
        _, inventory = receipt(transport, target, 'GET', path, cfg)
        validate_inventory(inventory, expected)
        for _ in range(sample_count(cfg, 'inventory_samples', quick)):
            observe('inventory_warm', 'GET', path, lambda value: validate_inventory(value, expected))
        status = settled(transport, target, cfg)
        validate_assets(status)
        (root,) = [asset for asset in status['assets'] if asset['kind'] == 'rootfs']
        original = Path(root['path']).resolve(strict=True)
        assert Path(root['path']).is_symlink(), 'materialization requires an isolated linked original'
        remove_owned_asset(service.assets_dir, root['path'])
        _, missing = receipt(transport, target, 'GET', '/assets/status', cfg)
        assert missing['ready'] is False and any(asset['kind'] == 'rootfs' and asset['status'] == 'missing' for asset in missing['assets'])
        def started_reconciliation(value):
            assert value['started'] is True, 'ensure did not start'
        for case, count in (('ensure_materialize', 1), ('ensure_ready', 0)):
            started = time.monotonic_ns()
            observe(case, 'POST', '/assets/ensure', started_reconciliation)
            status = settled(transport, target, cfg)
            elapsed = (time.monotonic_ns() - started) / 1_000_000
            validate_assets(status, downloaded=count)
            restored = next(asset for asset in status['assets'] if asset['kind'] == 'rootfs')
            assert filecmp.cmp(original, restored['path'], shallow=False), 'materialized bytes differ from pinned local source'
            metrics.setdefault(f'{target.label}.{keys[case]}.completion_ms', {'unit': 'milliseconds', 'samples': []})['samples'].append(elapsed)
    finally:
        transport.close()
    for case, observations in collected.items():
        for key, value in observations.metrics(f'{target.label}.{keys[case]}.', len(observations.completion)).items():
            metrics.setdefault(key, {'unit': value['unit'], 'samples': []})['samples'].extend(value['samples'])


def collect_images(quick: bool) -> dict:
    """Preparation and fixture startup are excluded from all endpoint receipts."""
    from .route_image_fixture import image_target, prepared_registry
    cfg = image_config()
    metrics: dict = {}
    with prepared_registry(cfg) as fixture:
        reference, certificate, requests, expected = fixture
        for label in ('service', 'gateway'):
            for cycle in range(sample_count(cfg, 'cycles', quick)):
                with image_target(label, reference, certificate) as (target, service):
                    assert service.proc is not None
                    from psutil import Process
                    measure(metrics, f'{label}.image_cycle.{cycle}', Process(service.proc.pid),
                        lambda: sample_images(target, service, reference, certificate, requests, expected, cfg, quick, metrics))
    require_image_metrics(metrics, cfg, quick)
    return metrics


def require_image_metrics(metrics: dict, cfg: dict, quick: bool) -> None:
    for label in ('service', 'gateway'):
        for cycle in range(sample_count(cfg, 'cycles', quick)):
            key = f'{label}.image_cycle.{cycle}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing finite resource cycle')
        for operation, endpoint in cfg['endpoints'].items():
            for case, key in endpoint['cases'].items():
                count = sample_count(cfg, 'cycles', quick) * (1 if key == 'cycles' else sample_count(cfg, key, quick))
                for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms'):
                    metric = f'{label}.{operation} [{case}].{suffix}'
                    if metric not in metrics or len(metrics[metric]['samples']) != count:
                        raise ValueError(f'missing or partial image metric: {metric}')
                if case.startswith('ensure_'):
                    assert len(metrics[f'{label}.{operation} [{case}].completion_ms']['samples']) == count
