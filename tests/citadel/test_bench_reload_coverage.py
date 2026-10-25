"""Reload cannot inherit desired-file or idle/read performance coverage."""
import copy

import pytest

from tests.citadel.test_bench_route_coverage import config, registered_operations, validate

RATIONALE = 'Corporate reload success needs exact live-owner acknowledgement and independent effective-policy traffic; file echo and idle reload are not that workload.'


def test_reload_is_one_finite_live_mutation_with_managed_states_explicit():
    operation = 'POST /corp/reload'
    coverage = copy.deepcopy(config())
    assert set(coverage['reload_controls']['endpoints']) == {operation}, RATIONALE
    assert coverage['reload_controls']['endpoints'][operation]['cases'] == {'block-original':'cycles','restore-original':'cycles'}, RATIONALE
    assert operation+' [managed-lease-retirement]' in coverage['unmeasured_cases'], RATIONALE
    del coverage['reload_controls']['endpoints'][operation]
    with pytest.raises(AssertionError,match='Missing'):
        validate(registered_operations(),coverage)
    coverage = copy.deepcopy(config())
    coverage['measured'].append({'method':'POST','path':'/corp/reload','case':'fake-concurrent-mutation'})
    with pytest.raises(AssertionError):
        validate(registered_operations(),coverage)
