"""Finite image operations require actual bytes and isolated mutation ownership."""
import copy

import pytest

from benchmarks.helpers import route_images


def test_finite_operations_are_not_concurrent_poll_cases():
    cfg = route_images.image_config()
    assert set(cfg['endpoints']) == {'GET /images', 'GET /images/{name}/inventory', 'POST /images/pull', 'POST /assets/ensure'}
    assert cfg['quick_cycles'] < cfg['cycles'] and cfg['warm_pulls'] < 10
    assert cfg['cached_samples'] > 1 and cfg['inventory_samples'] > 1


def test_catalog_requires_exact_pin_inventory_and_obom():
    reference = '127.0.0.1:12345/library/dev@sha256:' + '1' * 64
    payload: dict = {'catalog': {'reference': '127.0.0.1:12345/library/catalog:stable', 'digest': 'sha256:' + '2' * 64, 'channel': 'stable'}, 'images': [{'name': 'dev', 'image': reference, 'cached': 'ready', 'app': {'image': reference, 'architecture': 'arm64', 'evidence': [{'kind': kind, 'reference': reference, 'media_type': 'test'} for kind in ('obom', 'package_inventory')]}}]}
    route_images.validate_catalog(payload, reference, 'arm64')
    for field in ('image', 'app'):
        bad = copy.deepcopy(payload)
        bad['images'][0][field] = None
        with pytest.raises((AssertionError, TypeError)):
            route_images.validate_catalog(bad, reference, 'arm64')
    bad = copy.deepcopy(payload)
    bad['images'][0]['app']['evidence'].pop()
    with pytest.raises(AssertionError):
        route_images.validate_catalog(bad, reference, 'arm64')


def test_inventory_compares_complete_producer_versions():
    expected: dict = {'schema': 'capsem.image.packages.v1', 'subject': 'local@pin', 'architecture': 'arm64', 'packages': [{'name': 'git', 'version': '2.50', 'architecture': 'arm64'}], 'tools': [{'name': 'gh', 'version': '2.0'}]}
    payload: dict = {'image': 'local@pin', 'architecture': 'arm64', 'artifact_reference': 'local@evidence', 'inventory': expected}
    route_images.validate_inventory(payload, expected)
    bad = copy.deepcopy(payload)
    bad['inventory']['packages'][0]['version'] = 'wrong'
    with pytest.raises(AssertionError):
        route_images.validate_inventory(bad, expected)
    bad['inventory'] = None
    with pytest.raises(AssertionError):
        route_images.validate_inventory(bad, expected)


def test_inventory_uses_public_tool_projection_of_full_producer_artifact():
    produced = {
        'schema': 'capsem.package_inventory.v1', 'subject': 'local@pin', 'architecture': 'arm64',
        'packages': [{'name': 'git', 'version': '2.50', 'architecture': 'arm64'}],
        'tools': [{'name': 'gh', 'version': '2.0', 'binary_path': 'usr/bin/gh', 'binary_sha256': 'a' * 64}],
    }
    public = {
        'schema': 'capsem.package_inventory.v1', 'subject': 'local@pin', 'architecture': 'arm64',
        'packages': [{'name': 'git', 'version': '2.50', 'architecture': 'arm64'}],
        'tools': [{'name': 'gh', 'version': '2.0'}],
    }
    payload = {'image': 'local@pin', 'architecture': 'arm64',
               'artifact_reference': 'local@evidence', 'inventory': public}
    route_images.validate_inventory(payload, produced)
    for field in ('packages', 'tools'):
        for changed in ([], [{**public[field][0], 'version': 'wrong'}]):
            bad = copy.deepcopy(payload)
            bad['inventory'][field] = changed
            with pytest.raises(AssertionError):
                route_images.validate_inventory(bad, produced)


def test_cached_and_cold_transfer_claims_require_actual_registry_receipts():
    route_images.require_transfers(['/v2/library/dev/blobs/sha256:abc', '/v2/library/dev/blobs/sha256:def'], cold=True)
    route_images.require_transfers(['/v2/library/dev/manifests/sha256:abc'], cold=False)
    with pytest.raises(AssertionError):
        route_images.require_transfers([], cold=True)
    with pytest.raises(AssertionError):
        route_images.require_transfers(['/v2/library/dev/blobs/sha256:abc'], cold=False)


def test_asset_removal_cannot_follow_shared_symlink(tmp_path):
    source = tmp_path / 'shared'
    source.mkdir()
    real = source / 'rootfs'
    real.write_bytes(b'original')
    owned = tmp_path / 'owned'
    owned.mkdir()
    link = owned / 'rootfs'
    link.symlink_to(real)
    route_images.remove_owned_asset(owned, str(link))
    assert not link.exists() and real.read_bytes() == b'original'
    with pytest.raises(ValueError, match='owned'):
        route_images.remove_owned_asset(owned, str(real))
    with pytest.raises(ValueError):
        route_images.remove_owned_asset(owned, str(owned / '../shared/rootfs'))


def test_assets_require_completion_count_and_actual_files(tmp_path):
    asset = tmp_path / 'rootfs'
    asset.write_bytes(b'abc')
    payload: dict = {'ready': True, 'downloading': False, 'errors': [], 'downloaded': 1, 'assets': [{'name': 'rootfs.erofs', 'kind': 'rootfs', 'status': 'present', 'path': str(asset), 'expected_size': 3, 'actual_size': 3, 'expected_hash': '1' * 64}]}
    route_images.validate_assets(payload, downloaded=1)
    with pytest.raises(AssertionError):
        route_images.validate_assets(payload, downloaded=0)
    asset.unlink()
    with pytest.raises(AssertionError):
        route_images.validate_assets(payload, downloaded=1)


def test_partial_or_missing_finite_receipts_fail_loudly():
    cfg = route_images.image_config()
    metrics = {}
    for label in ('service', 'gateway'):
        for cycle in range(cfg['quick_cycles']):
            metrics[f'{label}.image_cycle.{cycle}.wall_s'] = {'samples': [1.0]}
        for operation, endpoint in cfg['endpoints'].items():
            for case, key in endpoint['cases'].items():
                count = cfg['quick_cycles'] * (1 if key == 'cycles' else cfg['quick_' + key])
                for suffix in ('latency_ms', 'headers_ms', 'first_body_byte_ms', *(('completion_ms',) if case.startswith('ensure_') else ())):
                    metrics[f'{label}.{operation} [{case}].{suffix}'] = {'samples': [1.0] * count}
    route_images.require_image_metrics(metrics, cfg, True)
    del metrics['gateway.POST /images/pull [pull_cold].first_body_byte_ms']
    with pytest.raises(ValueError, match='partial'):
        route_images.require_image_metrics(metrics, cfg, True)


def test_strict_image_receipt_refuses_retry_error_and_empty_body():
    from types import SimpleNamespace

    from helpers.http_measurement import MeasuredResponse
    cfg = route_images.image_config()
    target = SimpleNamespace(headers={})
    def response(**changed):
        fields: dict = {'status': 200, 'headers': {}, 'body': b'{"image":"dev","resolved":"local@pin","digest":"sha256:abc"}', 'headers_ns': 1, 'first_body_byte_ns': 2, 'complete_ns': 3, 'attempts': 1}
        fields.update(changed)
        return SimpleNamespace(measure_request=lambda *args, **kwargs: MeasuredResponse(**fields))
    route_images.receipt(response(), target, 'POST', '/images/pull', cfg, {'image': 'dev'})
    for changed in ({'status': 403}, {'attempts': 2}, {'body': b'', 'first_body_byte_ns': None}):
        with pytest.raises((AssertionError, ValueError)):
            route_images.receipt(response(**changed), target, 'POST', '/images/pull', cfg)


def test_typed_finite_counts_reject_unbounded_mutation_and_missing_owner():
    from capsem_builder.gate.benchmarkimageschema import ImageRouteConfig
    cfg = route_images.image_config()
    ImageRouteConfig.model_validate(cfg)
    for changed in ({'cycles': 999}, {'timeout_seconds': float('inf')}, {'warm_pulls': 0}):
        with pytest.raises(ValueError):
            ImageRouteConfig.model_validate({**cfg, **changed})


def test_owned_asset_rejects_symlink_parent_escape(tmp_path):
    shared = tmp_path / 'shared'
    shared.mkdir()
    real = shared / 'rootfs'
    real.write_bytes(b'keep')
    owned = tmp_path / 'owned'
    owned.mkdir()
    (owned / 'directory').symlink_to(shared)
    with pytest.raises(ValueError, match='owned'):
        route_images.remove_owned_asset(owned, str(owned / 'directory/rootfs'))
    assert real.read_bytes() == b'keep'


def test_complete_finite_sequence_checks_mutation_counts_and_retained_bytes(monkeypatch, tmp_path):
    """Dispatch unit proof only: synthetic transport, never native evidence."""
    import json
    from types import SimpleNamespace

    from helpers.http_measurement import MeasuredResponse
    cfg = route_images.image_config()
    reference = '127.0.0.1:12345/library/dev@sha256:' + '1' * 64
    inventory = {'schema': 'test', 'subject': reference, 'architecture': 'arm64', 'packages': [{'name': 'git', 'version': '1.0', 'architecture': 'arm64'}], 'tools': []}
    source = tmp_path / 'original'
    source.write_bytes(b'original')
    owned = tmp_path / 'owned'
    owned.mkdir()
    root = owned / 'rootfs'
    root.symlink_to(source)
    certificate = tmp_path / 'ca'
    certificate.write_text('fixture cert')
    requests, calls = [], []
    pulled = False
    downloaded = None
    closed = []
    target = SimpleNamespace(label='service', transport_factory=lambda: SimpleNamespace(close=lambda: closed.append(True)))
    def fake_receipt(transport, target, method, path, limits, body=None):
        nonlocal pulled, downloaded
        calls.append((method, path))
        if path == '/images?refresh=true':
            requests.append('/v2/library/catalog/manifests/stable')
        if path.startswith('/images/dev/inventory'):
            payload: dict = {'image': reference, 'architecture': 'arm64', 'artifact_reference': 'local@evidence', 'inventory': inventory}
        elif path.startswith('/images') and method == 'GET':
            payload: dict = {'catalog': {'reference': '127.0.0.1:12345/library/catalog:stable', 'digest': 'sha256:' + '2' * 64, 'channel': 'stable'}, 'images': [{'name': 'dev', 'image': reference, 'cached': 'ready' if pulled else 'missing', 'app': {'image': reference, 'architecture': 'arm64', 'evidence': [{'kind': kind, 'reference': reference, 'media_type': 'test'} for kind in ('obom', 'package_inventory')]}}]}
        elif path == '/images/pull':
            assert body == {'image': 'dev', 'registry': {'ca_pem': 'fixture cert'}}
            if not pulled:
                requests.extend(['/v2/library/dev/blobs/sha256:config', '/v2/library/dev/blobs/sha256:data'])
            pulled = True
            payload: dict = {'image': 'dev', 'resolved': reference, 'digest': reference.split('@')[1]}
        else:
            if method == 'POST':
                downloaded = 0 if root.exists() else 1
                if downloaded:
                    root.write_bytes(source.read_bytes())
            payload: dict = {'ready': root.exists(), 'downloading': False, 'errors': [] if root.exists() else ['missing'], 'downloaded': downloaded, 'started': True, 'assets': [{'name': 'rootfs.erofs', 'kind': 'rootfs', 'status': 'present' if root.exists() else 'missing', 'path': str(root), 'expected_size': len(b'original'), 'actual_size': len(b'original'), 'expected_hash': '1' * 64}]}
        return MeasuredResponse(200, {}, json.dumps(payload).encode(), 1, 2, 3), payload
    monkeypatch.setattr(route_images, 'receipt', fake_receipt)
    metrics = {}
    route_images.sample_images(target, SimpleNamespace(assets_dir=owned), reference, certificate, requests, inventory, cfg, True, metrics)
    assert sum(method == 'POST' and path == '/images/pull' for method, path in calls) == 1 + cfg['quick_warm_pulls']
    assert sum(method == 'POST' and path == '/assets/ensure' for method, path in calls) == 2
    assert root.read_bytes() == source.read_bytes() and not root.is_symlink()
    assert closed == [True]
    for case in ('pull_cold', 'pull_warm', 'ensure_materialize', 'ensure_ready'):
        assert any(f'[{case}].first_body_byte_ms' in key for key in metrics)


def test_catalogue_only_preparation_does_not_certify_inventory_or_blob_transfer():
    reference = '127.0.0.1:12345/library/dev@sha256:' + '1' * 64
    # Even a successful catalogue refresh is insufficient for the cold pull.
    with pytest.raises(AssertionError):
        route_images.require_transfers(['/v2/library/catalog/manifests/stable'], cold=True)
    with pytest.raises(AssertionError):
        route_images.validate_inventory({'image': reference, 'architecture': 'arm64', 'inventory': None}, {'subject': reference, 'architecture': 'arm64', 'packages': [], 'tools': []})
