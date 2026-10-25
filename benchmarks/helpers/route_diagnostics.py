"""Known synthetic JSONL input facts, not actual crashes or copied route output."""
from __future__ import annotations

import json
from datetime import UTC, datetime
from urllib.parse import parse_qs, urlsplit

MARKER = 'SYNTHETIC-LOG-INPUT'


def config():
    from capsem_builder.gate.benchmarknetworkschema import DiagnosticCorpusConfig

    from .route_fixtures import corpus_config
    return DiagnosticCorpusConfig.model_validate(corpus_config()['diagnostics']).model_dump()


def records(cfg):
    def event(ts, level, message, location=None, target='synthetic-fixture'):
        fields = {'message': MARKER + ' ' + message}
        if location is not None:
            fields['location'] = location
        return {'timestamp': ts, 'level': level, 'target': target, 'fields': fields}
    return [event(cfg['old_timestamp'], 'ERROR', 'panic excluded-old', '/synthetic/old.rs:1'),
        event(cfg['first_timestamp'], 'ERROR', 'panic fixture-one', '/synthetic/fixture.rs:1'),
        event(cfg['second_timestamp'], 'ERROR', 'panic fixture-two', '/synthetic/fixture.rs:2'),
        event(cfg['warning_timestamp'], 'WARN', 'ipc fixture-warning', target='ipc')]


def seed(service, cfg):
    from capsem_builder.gate.benchmarknetworkschema import DiagnosticCorpusConfig
    cfg = DiagnosticCorpusConfig.model_validate(cfg).model_dump()
    assert datetime.fromisoformat(cfg['since']) > datetime.now(UTC), 'synthetic since window expired'
    logs = service.home_dir / 'logs'
    assert not logs.is_symlink(), 'refusing existing app log symlink'
    logs.mkdir(exist_ok=True)
    assert not list(logs.glob('*.jsonl')), 'refusing existing app log generation'
    output = bytearray()
    padding = {'timestamp': cfg['old_timestamp'], 'level': 'INFO', 'target': 'synthetic-fixture',
        'fields': {'message': MARKER + ' ' + 'x' * cfg['padding_bytes']}}
    for index in range(4 + cfg['padding_lines']):
        record = records(cfg)[index] if index < 4 else padding
        line = (json.dumps(record) + '\n').encode()
        if len(output) + len(line) > cfg['max_file_bytes']:
            raise ValueError('synthetic diagnostic file budget exceeded')
        output.extend(line)
    path = logs / 'synthetic-diagnostics.jsonl'
    with path.open('xb') as stream:
        stream.write(output)
    path.chmod(0o600)


def expected_payload(path, query):
    cfg = config()
    pairs = {(cfg['since'], cfg['limit']), (cfg['since'], cfg['narrow_limit']), (cfg['later_since'], cfg['limit'])}
    if set(query) != {'since', 'limit'} or (query['since'], int(query['limit'])) not in pairs:
        raise ValueError('unreviewed diagnostic window/limit')
    if path not in {'/panics', '/triage'}:
        raise ValueError('unreviewed diagnostic operation')
    limit = int(query['limit'])
    # Independent generator facts; not a parse of the served log/response.
    selected = [row for row in records(cfg) if row['timestamp'] >= query['since']]
    panics: list[dict] = [{'ts': row['timestamp'], 'binary': 'capsem-app', 'thread': None,
        'location': row['fields']['location'], 'message': row['fields']['message'], 'frames': []}
        for row in selected if row['level'] == 'ERROR'][:limit]
    if path == '/panics':
        return {'panics': panics}
    errors = [{'ts': row['timestamp'], 'binary': 'capsem-app', 'level': row['level'],
        'target': row['target'], 'message': row['fields']['message']} for row in selected][:limit]
    rank = [f"panic {row['ts'][:19]} in capsem-app at {row['location']} -- {row['message']}" for row in panics]
    rank.extend(f"ipc-warn {row['ts'][:19]} in capsem-app -- {row['message']}" for row in errors if row['target'] == 'ipc')
    return {'since': query['since'], 'session_id': None, 'host': {'panics': panics, 'errors': errors, 'slow_ops': []}, 'session': {}, 'rank': rank}


def validate_response(path, body):
    from .route_validation import SCHEMAS
    split = urlsplit(path)
    query = parse_qs(split.query, strict_parsing=True)
    if any(len(values) != 1 for values in query.values()):
        raise ValueError('duplicate diagnostic query')
    expected = expected_payload(split.path, {key: values[0] for key, values in query.items()})
    SCHEMAS[split.path].validate_json(body)
    assert json.loads(body) == expected, 'diagnostic fields/order/since/limit/rank differ from synthetic input'
