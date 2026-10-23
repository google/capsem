"""Image measurement setup owns fresh homes; no subprocesses in these tests."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_image_fixture as fixture
from benchmarks.helpers import route_requests


def test_independent_service_homes_keep_real_source_assets_untouched(monkeypatch, tmp_path):
    source = tmp_path / 'source'
    source.mkdir()
    manifest = '{"format":2,"asset_base":"https://never-fetch.invalid"}'
    (source / 'manifest.json').write_text(manifest)
    (source / 'rootfs').write_bytes(b'pinned-original')
    monkeypatch.setattr(fixture, 'ASSETS_DIR', source)
    calls, homes = [], []
    def service():
        home = tmp_path / str(len(homes))
        home.mkdir()
        homes.append(home)
        def start():
            settings = (home / 'settings.toml').read_text()
            assert 'catalog_ca' in settings and 'admit' not in settings
            assert json.loads((home / 'benchmark-assets/manifest.json').read_text()) == {'format': 2}
            metadata = json.loads((home / 'benchmark-assets/manifest-metadata.json').read_text())
            assert metadata['manifest_url'] == (source / 'manifest.json').as_uri()
            calls.append('start')
        return SimpleNamespace(home_dir=home, proc=SimpleNamespace(pid=123), start=start, stop=lambda: calls.append('stop'))
    monkeypatch.setattr(fixture, 'ServiceInstance', service)
    monkeypatch.setattr(route_requests, 'Process', lambda *_: None)
    for _ in range(2):
        with fixture.image_target('service', '127.0.0.1:12345/library/dev@sha256:pin', tmp_path / 'ca') as (target, owner):
            assert target.label == 'service'
            assert (owner.assets_dir / 'rootfs').is_symlink()
    assert len(set(homes)) == 2 and calls == ['start', 'stop', 'start', 'stop']
    assert (source / 'manifest.json').read_text() == manifest
    assert (source / 'rootfs').read_bytes() == b'pinned-original'


def test_preexisting_cache_refuses_cold_claim_before_child(monkeypatch, tmp_path):
    source = tmp_path / 'source'
    source.mkdir()
    (source / 'manifest.json').write_text('{}')
    home = tmp_path / 'home'
    home.mkdir()
    (home / 'cache').mkdir()
    stopped = []
    def forbidden():
        raise AssertionError('must not start a child on a reused home')
    monkeypatch.setattr(fixture, 'ASSETS_DIR', source)
    monkeypatch.setattr(fixture, 'ServiceInstance', lambda: SimpleNamespace(home_dir=home, start=forbidden, stop=lambda: stopped.append(True)))
    with pytest.raises(AssertionError), fixture.image_target('service', '127.0.0.1:12345/library/dev@sha256:pin', tmp_path / 'ca'):
        raise AssertionError('must not yield a contaminated owner')
    assert stopped == [True]
