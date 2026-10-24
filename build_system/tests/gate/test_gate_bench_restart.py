"""Restart owns one native step; normal benchmarks retain confinement."""
import pytest
from capsem_builder.gate import bench, host
from test_gate_bench import CONFIG, _argv, _plan, _step


@pytest.mark.parametrize('mac',[True,False])
def test_full_native_split_covers_union_without_elevating_routes(monkeypatch,mac):
    monkeypatch.setattr(host,'on_macos',lambda:mac)
    plan = _plan(bench.BenchCommand,commit='exact-source')
    ordinary = _argv(plan,'bench.run')
    native = _argv(plan,'native.bench.restart')
    assert ordinary[-2:] == ['--exclude','restart']
    assert 'outside kernel sandbox' not in _step(plan,'bench.run').actions[0].render()
    assert 'outside kernel sandbox' in _step(plan,'native.bench.restart').actions[0].render()
    assert 'exact-source' in ordinary and 'exact-source' in native
    assert 'restart' in native and 'routes' not in native
    assert ('tests.helpers.run_native_restart' in native) is (not mac)
    assert str(CONFIG.path(CONFIG.benchmark.run.store)) in native


def test_explicit_restart_only_mixed_and_quick_selection():
    only = _plan(bench.BenchCommand,dimensions='restart')
    assert 'bench.run' not in only.labels and 'native.bench.restart' in only.labels
    mixed = _plan(bench.BenchCommand,dimensions='routes restart')
    assert _argv(mixed,'bench.run')[-1] == 'routes'
    assert 'native.bench.restart' in mixed.labels
    quick = _plan(bench.BenchCommand,quick=True)
    assert 'native.bench.restart' not in quick.labels
