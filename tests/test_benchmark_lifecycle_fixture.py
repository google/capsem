"""Source-only ownership tests: every product/image dependency is replaced."""
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_lifecycle_fixture as fixture


def test_fixture_validates_existing_pin_then_grants_exact_isolated_owner(monkeypatch, tmp_path):
    layout = tmp_path / 'layout'
    layout.mkdir()
    home = tmp_path / 'home'
    home.mkdir()
    calls = []
    reference = '127.0.0.1/library/dev@sha256:' + 'a' * 64
    certificate = tmp_path / 'certificate'
    image = SimpleNamespace(ready=lambda: layout, settings=lambda: SimpleNamespace(name='dev'))
    @contextmanager
    def registry(path, selected, name):
        assert selected == layout and name == 'dev'
        calls.append('registry')
        yield reference, certificate, []
        calls.append('registry-close')
    service = SimpleNamespace(home_dir=home, start=lambda: calls.append('start'), stop=lambda: calls.append('stop'))
    def create(**kwargs):
        assert kwargs == {'isolated_corp_config': True}
        return service
    monkeypatch.setattr(fixture, 'ServiceInstance', create)
    def grant(owner, selected, **kwargs):
        assert owner == home and selected == reference
        assert kwargs == {'catalog': '127.0.0.1/library/catalog:stable', 'certificate': certificate}
        calls.append('grant')
    monkeypatch.setattr(fixture, 'fixture_inputs', lambda: (image, lambda value: calls.append(('verify', value)), grant, registry))
    monkeypatch.setattr(fixture, 'service_target', lambda owner, label: (owner, label))
    with (
        pytest.raises(RuntimeError, match='synthetic failure'),
        fixture.lifecycle_target('gateway') as (target, owner, selected, ca),
    ):
        assert target == (service, 'gateway') and owner is service
        assert selected == reference and ca == certificate
        raise RuntimeError('synthetic failure')
    # Contextmanager body failure still closes service; registry outer owner unwinds.
    assert calls[:4] == [('verify', layout), 'registry', 'grant', 'start']
    assert 'stop' in calls


def test_missing_immutable_image_fails_before_service_or_registry(monkeypatch):
    def unavailable():
        raise FileNotFoundError('exact pinned image missing')
    monkeypatch.setattr(fixture, 'fixture_inputs', lambda: (SimpleNamespace(ready=unavailable), None, None, None))
    monkeypatch.setattr(fixture, 'ServiceInstance', lambda **_: pytest.fail('service launched'))
    with pytest.raises(FileNotFoundError, match='pinned'), fixture.lifecycle_target('service'):
        pytest.fail('missing image admitted')
