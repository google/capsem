"""Exact method/scope/debt classifications cannot launder UDS404 or rendering."""
import copy
from pathlib import Path

import pytest

from tests.citadel.test_bench_route_coverage import config, registered_operations, validate


def test_gateway_only_mints_cannot_disappear_overlap_or_become_uds_coverage():
    original = config()
    operations = original['preview_controls']['endpoints']
    assert len(operations) == 5
    for operation in operations:
        changed = copy.deepcopy(original)
        del changed['preview_controls']['endpoints'][operation]
        with pytest.raises(AssertionError,match='Missing'):
            validate(registered_operations(),changed)
    for operation in ('POST /vms/{id}/surface/session','POST /vms/{id}/exposures/{exposure_id}/preview-session'):
        changed = copy.deepcopy(original)
        changed['preview_controls']['endpoints'][operation]['target_scope'] = 'both'
        with pytest.raises(AssertionError):
            validate(registered_operations(),changed)
        changed = copy.deepcopy(original)
        method,path = operation.split(' ',1)
        changed['measured'].append({'method':method,'path':path,'case':'fake-poll'})
        with pytest.raises(AssertionError):
            validate(registered_operations(),changed)
    assert 'POST /vms/{id}/surface/session [actual-xpra-native-render]' in original['unmeasured_cases']
    assert 'DELETE /vms/{id}/exposures/{exposure_id}/preview-session [physical-preview-ledger]' in original['unmeasured_cases']


def test_bootstrap_expiry_and_responder_contract_stay_canonical():
    root = Path(__file__).parents[1]
    assert 'pub const PREVIEW_BOOTSTRAP_LIFETIME_SECS: u16 = 30;' in (root/'crates/capsem-proto/src/lib.rs').read_text()
    from tests.ironbank.kingslanding.test_preview import SERVER
    from tests.ironbank.kingslanding.test_workload_surface import IMAGE_CONFIG, PORT
    assert PORT == 14500 and IMAGE_CONFIG['Labels'] == {'org.capsem.surface':'xpra','org.capsem.surface.port':'14500'}
    command = IMAGE_CONFIG['Cmd']
    assert isinstance(command,list) and 'exec nc -lk -p 14500' in command[0]
    assert 'protocol_version = "HTTP/1.1"' in SERVER.read_text()
    assert 'self.body(200, b"redirected")' in SERVER.read_text()
