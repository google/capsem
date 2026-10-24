"""A real check is not a plan, arbitrary subprocess or native apply proof."""
import copy

import pytest

from tests.citadel.test_bench_route_coverage import config, registered_operations, validate


def test_update_check_has_exact_finite_case_and_apply_stays_owned_debt():
    coverage = copy.deepcopy(config())
    assert set(coverage['update_check']['endpoints']) == {'POST /update/check'}
    assert coverage['update_check']['endpoints']['POST /update/check']['cases'] == {'real-cli-manifest-fetch':'cycles'}
    assert 'POST /update/apply' in coverage['unmeasured']
    assert not any(case['path'] in {'/update/check','/update/apply'} for case in coverage['measured'])
    del coverage['update_check']['endpoints']['POST /update/check']
    with pytest.raises(AssertionError,match='Missing'):
        validate(registered_operations(),coverage)
