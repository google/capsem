"""The live publication audit compares both directions, including platforms."""
import copy

import pytest

from images.ci.registry_catalog import coverage_errors

REGISTRY = 'ghcr.io/google/capsem'
PLATFORMS = {'linux/amd64', 'linux/arm64'}
REF = REGISTRY + '/app@sha256:' + '1' * 64


def state():
    document = {'entries': {'app': {'versions': [{'image': REF, 'platforms': sorted(PLATFORMS)}]}}}
    return document, {'app'}, {REF: PLATFORMS}


def test_matching_registry_and_catalog():
    document, repositories, published = state()
    assert coverage_errors(document, repositories, published, {'app'}) == []


@pytest.mark.parametrize('defect', ['registry orphan', 'catalog ghost', 'missing manifest', 'missing amd64', 'missing arm64', 'unlisted platform', 'source omission', 'wrong repository'])
def test_mismatches_fail(defect):
    document, repositories, published = copy.deepcopy(state())
    expected = {'app'}
    if defect == 'registry orphan':
        repositories.add('orphan')
    elif defect == 'catalog ghost':
        repositories.clear()
    elif defect == 'missing manifest':
        published[REF] = None
    elif defect.startswith('missing '):
        published[REF] = PLATFORMS - {'linux/' + defect.split()[1]}
    elif defect == 'unlisted platform':
        published[REF] = PLATFORMS | {'linux/riscv64'}
    elif defect == 'wrong repository':
        other = REF.replace('/app@', '/another@')
        document['entries']['app']['versions'][0]['image'] = other
        published[other] = PLATFORMS
    else:
        expected.add('new-app')
    assert coverage_errors(document, repositories, published, expected), defect


def test_an_old_single_arch_version_is_retained_but_the_current_version_must_support_both():
    document, repositories, published = state()
    document['entries']['app']['versions'][0]['platforms'] = ['linux/arm64']
    published[REF] = {'linux/arm64'}
    assert coverage_errors(document, repositories, published, {'app'})
    newer = REF[:-64] + '2' * 64
    document['entries']['app']['versions'].append({'image': newer, 'platforms': sorted(PLATFORMS)})
    published[newer] = PLATFORMS
    assert coverage_errors(document, repositories, published, {'app'}) == []


def test_index_claims_are_checked_against_the_real_child_configuration(monkeypatch):
    from images.ci import registry_catalog as audit

    child = REGISTRY + '/app@sha256:' + '2' * 64
    config = REGISTRY + '/app@sha256:' + '3' * 64
    documents = {
        REF: {'manifests': [{'digest': child.split('@')[1], 'platform': {'os': 'linux', 'architecture': 'amd64'}}]},
        child: {'config': {'digest': config.split('@')[1]}},
        config: {'os': 'linux', 'architecture': 'arm64'},
    }
    monkeypatch.setattr(audit, '_json', lambda *argv: documents[argv[-1]])
    audit.platforms.cache_clear()
    try:
        with pytest.raises(ValueError, match='index platform differs from child config'):
            audit.platforms(REF)
    finally:
        audit.platforms.cache_clear()
