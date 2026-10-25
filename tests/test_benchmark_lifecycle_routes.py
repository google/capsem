"""Finite lifecycle source proof; synthetic backend is never native evidence."""
from __future__ import annotations

import pytest

from benchmarks.helpers import route_lifecycle


def test_exact_six_lifecycle_operations_have_finite_counts():
    cfg = route_lifecycle.lifecycle_config()
    assert set(cfg['endpoints']) == {'POST /vms/create', 'POST /vms/{id}/exec', 'POST /vms/{id}/fork', 'POST /vms/{id}/start', 'POST /vms/{id}/stop', 'DELETE /vms/{id}/delete'}
    assert 0 < cfg['quick_cycles'] < cfg['cycles'] <= 10


@pytest.mark.parametrize('changes', [{'status': 'Stopped'}, {'persistent': False}, {'pid': 0}, {'id': 'other'}])
def test_running_requires_actual_owned_persistent_process(changes):
    payload = {'id': 'vm', 'status': 'Running', 'persistent': True, 'pid': 50}
    with pytest.raises(AssertionError):
        route_lifecycle.validate_running({**payload, **changes}, 'vm')


@pytest.mark.parametrize('output', ['CAPSEM-UID-MAP 0 0 4294967295\nproof', 'proof', 'CAPSEM-UID-MAP 0 100000 65536\nwrong'])
def test_workload_validation_refuses_vm_fallback_and_wrong_bytes(output):
    with pytest.raises(AssertionError):
        route_lifecycle.validate_exec({'exit_code': 0, 'truncated': False, 'stdout': {'encoding': 'utf8', 'data': output}, 'stderr': {'encoding': 'utf8', 'data': ''}}, 'proof')


def test_raw_metric_validator_rejects_partial_lifecycle_samples():
    with pytest.raises(ValueError, match='missing or partial'):
        route_lifecycle.require_lifecycle_metrics({}, route_lifecycle.lifecycle_config(), False)


@pytest.mark.parametrize('changes', [
    {'cycles': 0}, {'quick_cycles': 11}, {'timeout_seconds': float('inf')},
    {'max_response_bytes': 0}, {'marker': 'bad; shell'}, {'exec_timeout_seconds': 301},
])
def test_lifecycle_config_rejects_unbounded_or_unsafe_inputs(changes):
    from capsem_builder.gate.benchmarklifecycleschema import LifecycleRouteConfig
    with pytest.raises(ValueError):
        LifecycleRouteConfig.model_validate({**route_lifecycle.lifecycle_config(), **changes})
