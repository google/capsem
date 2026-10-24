"""Independent source doubles for disabled operations; no native timing proof."""
import json
import shutil
import time
from types import SimpleNamespace

import pytest
from helpers import stopped_workspace
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_pause_resume
from benchmarks.helpers import route_suspend_refusal as subject
from benchmarks.helpers.route_workload_controls import Cycle

LIVE = '123e4567-e89b-42d3-a456-426614174000'
BOOT = '123e4567-e89b-42d3-a456-426614174001'


@pytest.mark.parametrize('fault', ['', 'error', 'pid', 'retired', 'boot', 'generation',
                                   'created-checkpoint', 'lost-checkpoint', 'saved-status', 'retry'])
def test_refusal_requires_original_live_owner_and_saved_bytes(monkeypatch, tmp_path, fault):
    monkeypatch.setattr(stopped_workspace, 'manifest_asset_pins', lambda _: {})
    service = SimpleNamespace(tmp_dir=tmp_path, assets_dir=tmp_path/'assets')
    subject.prepare_saved_session(service)
    saved = tmp_path/'persistent'/subject.VM_ID
    registry = json.loads((tmp_path/'persistent_registry.json').read_text())
    assert registry['vms'][subject.VM_NAME]['suspended'] is True
    assert (saved/'owner/checkpoint.vzsave').read_bytes() == subject.SAVED
    certificate = tmp_path/'certificate'
    certificate.write_text('synthetic fixture CA')
    live = tmp_path/'persistent'/LIVE
    rows = {subject.VM_ID: {'id': subject.VM_ID, 'pid': 0, 'status': 'Incompatible',
        'persistent': True, 'can_resume': False, 'available_actions': ['delete']}}
    state = {'alive': True, 'paused': False}
    cycle = Cycle(None, SimpleNamespace(label='service'), service, route_pause_resume.pause_resume_config(), {})

    def call(method, path, body=None, *, deadline, expected_status=200):
        assert deadline > time.monotonic()
        value = {'success': True}
        if path == '/vms/create':
            assert isinstance(body, dict)
            assert body['container']['image'] == 'pinned' and body['persistent'] is True
            (live/'owner').mkdir(parents=True)
            rows[LIVE] = {'id': LIVE, 'pid': 123, 'status': 'Running', 'persistent': True,
                'can_resume': False, 'available_actions': ['stop', 'fork', 'delete']}
            value = {'id': LIVE}
        elif path.endswith(('/pause', '/resume')):
            assert expected_status == 501
            value = {'error': 'other refusal' if fault == 'error' else 'warm suspend is disabled'}
            if path.endswith('/pause'):
                state['paused'] = True
                if fault == 'pid':
                    rows[LIVE]['pid'] = 124
                if fault == 'retired':
                    state['alive'] = False
                if fault == 'created-checkpoint':
                    (live/'owner/checkpoint.vzsave.complete').write_bytes(b'ok\n')
            elif fault == 'lost-checkpoint':
                (saved/'owner/checkpoint.vzsave').unlink()
            elif fault == 'saved-status':
                rows[subject.VM_ID]['status'] = 'Running'
        elif path.endswith('/stop'):
            state['alive'] = False
            value['persistent'] = True
        elif method == 'DELETE':
            vm_id = path.split('/')[2]
            shutil.rmtree(tmp_path/'persistent'/vm_id)
            del rows[vm_id]
        else:
            raise AssertionError((method, path))
        return MeasuredResponse(expected_status, {}, json.dumps(value).encode(), 1, 2, 3,
            attempts=2 if fault == 'retry' and path.endswith('/pause') else 1), value

    def read(path, **kwargs):
        return {'sandboxes': list(rows.values())} if path == '/vms/list' else dict(rows[path.split('/')[2]])

    monkeypatch.setattr(cycle, 'call', call)
    monkeypatch.setattr(cycle, 'read', read)
    monkeypatch.setattr(cycle, 'ready', lambda vm_id, *a, **kw: cycle.info(vm_id))
    monkeypatch.setattr(cycle, 'session', lambda *a, **kw: live)
    monkeypatch.setattr(subject, 'Process', lambda pid: SimpleNamespace(pid=pid, is_running=lambda: state['alive']))
    monkeypatch.setattr(subject, 'generation', lambda *a: 'changed' if state['paused'] and fault == 'generation' else 'original')
    monkeypatch.setattr(subject, 'vm_text', lambda *a: LIVE if state['paused'] and fault == 'boot' else BOOT)
    if fault:
        with pytest.raises((AssertionError, ValueError, FileNotFoundError)):
            subject.sample_disabled_suspend(cycle, 'pinned', certificate)
    else:
        subject.sample_disabled_suspend(cycle, 'pinned', certificate)
        assert not rows and not cycle.owned and len(cycle.metrics) == 8
        assert all('disabled-' in key for key in cycle.metrics)
