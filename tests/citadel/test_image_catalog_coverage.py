"""Image inventory, architecture builds and live registry audit are separate rails."""
from __future__ import annotations

import copy
import tomllib
from pathlib import Path

import pytest
import yaml

from images.ci.registry_catalog import INFRASTRUCTURE

ROOT = Path(__file__).resolve().parents[2]
CATALOG_RATIONALE = (
    'An official workload missing from the catalog disappears from the UI. '
    'Source images, catalog metadata and both publication matrices must agree.'
)
ARCHITECTURE_RATIONALE = (
    'Every official image must build for amd64 and arm64. An excluded architecture '
    'silently hides the application on that host; Claude Desktop once did this.'
)
REGISTRY_RATIONALE = (
    'Source agreement cannot prove publication. Audit the actual GHCR inventory '
    'and pinned manifests in both directions, for every declared platform, before '
    'publishing any catalog. Registry/network errors must fail publication.'
)
AUDIT_COMMAND = 'uv run --project build_system --frozen python -m images.ci.registry_catalog --catalog catalog/catalog.json --registry "$REGISTRY"'


def workflow():
    return yaml.safe_load((ROOT / '.github/workflows/images.yaml').read_text())


def inventory_errors(document, source, catalog):
    jobs = document['jobs']
    builds = set(jobs['image-arch']['strategy']['matrix']['image'])
    indexes = set(jobs['image']['strategy']['matrix']['image'])
    return [] if source == catalog == builds == indexes else [CATALOG_RATIONALE]


def architecture_errors(document):
    matrix = document['jobs']['image-arch']['strategy']['matrix']
    if set(matrix['arch']) != {'amd64', 'arm64'} or matrix.get('exclude'):
        return [ARCHITECTURE_RATIONALE]
    for row in matrix.get('include', []):
        if 'arch' in row and row['arch'] not in {'amd64', 'arm64'}:
            return [ARCHITECTURE_RATIONALE]
        if 'image' in row and row['image'] not in matrix['image']:
            return [ARCHITECTURE_RATIONALE]
    return []


def audit_errors(document):
    steps = document['jobs']['catalog']['steps']
    audit = [i for i, step in enumerate(steps) if step.get('run', '').strip() == AUDIT_COMMAND]
    publish = [i for i, step in enumerate(steps) if 'oras push' in step.get('run', '')]
    if len(audit) != 1 or not publish or audit[0] >= min(publish):
        return [REGISTRY_RATIONALE]
    step = steps[audit[0]]
    if step.get('if') or step.get('continue-on-error') or step.get('env', {}).get('GH_TOKEN') != '${{ github.token }}':
        return [REGISTRY_RATIONALE]
    return []


def test_only_documented_non_product_repositories_are_exempt():
    assert {'base', 'catalog', 'capsem-debug'} == INFRASTRUCTURE, CATALOG_RATIONALE


def test_every_source_image_is_in_the_catalog_and_both_publication_matrices():
    source = {path.parent.name for path in (ROOT / 'images').glob('*/Dockerfile')} - INFRASTRUCTURE
    catalog = set(tomllib.loads((ROOT / 'images/catalog.toml').read_text())['images'])
    assert not inventory_errors(workflow(), source, catalog), CATALOG_RATIONALE


def test_every_image_builds_for_both_host_architectures_without_exclusions():
    assert not architecture_errors(workflow()), ARCHITECTURE_RATIONALE


def test_live_registry_catalog_audit_is_required_before_publication():
    assert not audit_errors(workflow()), REGISTRY_RATIONALE


@pytest.mark.parametrize('side', ['source', 'catalog', 'build', 'index'])
def test_inventory_guard_rejects_a_missing_image_on_either_side(side):
    document = workflow()
    source = set(document['jobs']['image-arch']['strategy']['matrix']['image'])
    catalog = source.copy()
    if side == 'source':
        source.remove('antigravity-desktop')
    elif side == 'catalog':
        catalog.remove('antigravity-desktop')
    else:
        job = 'image-arch' if side == 'build' else 'image'
        document['jobs'][job]['strategy']['matrix']['image'].remove('antigravity-desktop')
    assert inventory_errors(document, source, catalog)


@pytest.mark.parametrize('arch', ['amd64', 'arm64'])
@pytest.mark.parametrize('defect', ['omit', 'exclude'])
def test_architecture_guard_rejects_missing_or_excluded_builds(arch, defect):
    document = workflow()
    matrix = document['jobs']['image-arch']['strategy']['matrix']
    if defect == 'omit':
        matrix['arch'].remove(arch)
    else:
        matrix['exclude'] = [{'image': 'claude-desktop', 'arch': arch}]
    assert architecture_errors(document)


@pytest.mark.parametrize('defect', ['removed', 'after publish', 'conditional', 'ignored failure', 'shell bypass'])
def test_registry_audit_cannot_be_skipped_or_neutralized(defect):
    document = copy.deepcopy(workflow())
    steps = document['jobs']['catalog']['steps']
    step = next(row for row in steps if row.get('run', '').strip() == AUDIT_COMMAND)
    if defect == 'removed':
        steps.remove(step)
    elif defect == 'after publish':
        steps.remove(step)
        steps.append(step)
    elif defect == 'conditional':
        step['if'] = 'false'
    elif defect == 'ignored failure':
        step['continue-on-error'] = True
    else:
        step['run'] += ' || true'
    assert audit_errors(document)
