"""Synthetic log input contracts; these tests cause no product crash or launch."""
import json
from copy import deepcopy
from types import SimpleNamespace
from urllib.parse import urlencode

import pytest
from helpers.http_measurement import MeasuredResponse
from helpers.http_transport import Transport

from benchmarks.helpers import route_diagnostics as subject
from benchmarks.helpers.route_cases import measured_cases
from benchmarks.helpers.route_fixtures import corpus_config, seed_diagnostics
from benchmarks.helpers.route_requests import Target, request
from benchmarks.helpers.route_validation import validate_wire


def test_static_corpus_is_exclusive_bounded_and_preserves_active_writers(tmp_path):
    service = SimpleNamespace(home_dir=tmp_path)
    cfg = corpus_config()['diagnostics']
    (tmp_path / 'logs').mkdir()
    active = tmp_path / 'logs' / 'service.log'
    active.write_bytes(b'live sink')
    seed_diagnostics(service, corpus_config())
    path = tmp_path / 'logs' / 'synthetic-diagnostics.jsonl'
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    assert len(rows) == 4 + cfg['padding_lines']
    assert rows[0]['timestamp'] == cfg['old_timestamp']
    assert all('SYNTHETIC-LOG-INPUT' in row['fields']['message'] for row in rows)
    assert path.stat().st_size <= cfg['max_file_bytes']
    assert path.stat().st_mode & 0o777 == 0o600
    assert active.read_bytes() == b'live sink'
    with pytest.raises(AssertionError, match='existing'):
        seed_diagnostics(service, corpus_config())


@pytest.mark.parametrize('kind', ['jsonl', 'symlink', 'directory', 'overflow'])
def test_log_seed_refuses_existing_app_generation_or_budget_overflow(tmp_path, kind):
    logs = tmp_path / 'logs'
    logs.mkdir()
    cfg = deepcopy(corpus_config())
    if kind == 'jsonl':
        (logs / 'active.jsonl').write_bytes(b'active')
    elif kind == 'symlink':
        (logs / 'active.jsonl').symlink_to(tmp_path / 'missing')
    elif kind == 'directory':
        (logs / 'active.jsonl').mkdir()
    else:
        cfg['diagnostics']['max_file_bytes'] = 1
    with pytest.raises((AssertionError, ValueError)):
        seed_diagnostics(SimpleNamespace(home_dir=tmp_path), cfg)
    assert not (logs / 'synthetic-diagnostics.jsonl').exists()


def test_six_cases_have_independent_since_limit_rank_and_schema_facts():
    cases = [case for case in measured_cases() if case.response == 'diagnostics']
    assert len(cases) == 6 and {case.path for case in cases} == {'/panics', '/triage'}
    for case in cases:
        expected = subject.expected_payload(case.path, case.query)
        assert validate_wire(case, json.dumps(expected).encode(), None) == expected
        assert expected['panics'] if case.path == '/panics' else expected['host']['panics']
    cfg = corpus_config()['diagnostics']
    complete = subject.expected_payload('/triage', {'since': cfg['since'], 'limit': cfg['limit']})
    assert len(complete['host']['panics']) == 2 and len(complete['host']['errors']) == 3
    assert len(complete['rank']) == 3 and complete['rank'][2].startswith('ipc-warn ')
    assert complete['host']['slow_ops'] == []  # app JSONL does not feed slow ops
    assert complete['host']['panics'][0]['location'] == '/synthetic/fixture.rs:1'
    assert 'fixture-one' in complete['rank'][0] and 'fixture-two' in complete['rank'][1]
    later = subject.expected_payload('/panics', {'since': cfg['later_since'], 'limit': cfg['limit']})
    assert len(later['panics']) == 1 and 'fixture-two' in later['panics'][0]['message']


@pytest.mark.parametrize('change', ['empty', 'order', 'rank', 'limit', 'type', 'since'])
def test_every_measured_receipt_rejects_wrong_content_not_only_preflight(change):
    case = next(case for case in measured_cases() if case.path == '/triage' and case.case == 'synthetic-populated')
    value = deepcopy(subject.expected_payload(case.path, case.query))
    if change == 'empty':
        value['host']['panics'] = []
    elif change == 'order':
        value['host']['panics'].reverse()
    elif change == 'rank':
        value['rank'].reverse()
    elif change == 'limit':
        value['host']['errors'].pop()
    elif change == 'type':
        value['host']['panics'][0]['frames'] = 'wrong'
    else:
        value['since'] = '30m'
    receipt = MeasuredResponse(200, {}, json.dumps(value).encode(), 1, 2, 3)
    class FixedTransport(Transport):
        def measure_request(self, *args, **kwargs):
            return receipt
    transport = FixedTransport(host='127.0.0.1', port=1)
    target = Target('service', lambda: transport, {}, {}, limits={'timeout_seconds': 1, 'max_response_bytes': 10000})
    with pytest.raises((AssertionError, ValueError)):
        request(transport, target, case.resolve(None), case.response)


def test_unreviewed_diagnostic_queries_cannot_silently_widen_corpus():
    cfg = corpus_config()['diagnostics']
    for query in ({'since': cfg['since'], 'limit': 200}, {'since': '1h', 'limit': 3}, {'since': cfg['since'], 'limit': 3, 'id': 'other'}):
        with pytest.raises(ValueError):
            subject.validate_response('/triage?' + urlencode(query), b'{}')


def test_queries_and_original_fixture_limits_remain_config_owned():
    from capsem_builder.gate.benchmarknetworkschema import DiagnosticCorpusConfig
    cfg = corpus_config()['diagnostics']
    DiagnosticCorpusConfig.model_validate(cfg)
    for changed in ({'padding_lines': 10001}, {'max_file_bytes': 1048577}, {'narrow_limit': 2}, {'later_since': cfg['since']}, {'since': '30m'}):
        with pytest.raises(ValueError):
            DiagnosticCorpusConfig.model_validate({**cfg, **changed})
    for case in (case for case in measured_cases() if case.response == 'diagnostics'):
        expected_limit = cfg['narrow_limit'] if case.case == 'synthetic-limited' else cfg['limit']
        expected_since = cfg['later_since'] if case.case == 'synthetic-later-since' else cfg['since']
        assert case.query == {'since': expected_since, 'limit': expected_limit}
