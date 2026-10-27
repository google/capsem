"""Refuse publication when GHCR and the application catalog disagree."""
from __future__ import annotations

import argparse
import functools
import json
import subprocess
from collections.abc import Mapping
from pathlib import Path

from images.ci.catalog import descriptions

SUPPORTED_PLATFORMS = frozenset({'linux/amd64', 'linux/arm64'})
# Explicit non-product repositories: the shared base, catalog OCI artifact,
# and test-only fixture. Admitting capsem-debug through the default product
# catalog would violate tests/images/test_image_ci.py DEBUG_IMAGE_RATIONALE.
INFRASTRUCTURE = {'base', 'catalog', 'capsem-debug'}


def coverage_errors(document, repositories, published, expected) -> list[str]:
    entries = document['entries']
    errors = []
    for name in sorted(repositories - set(entries)):
        errors.append(f'GHCR workload repository absent from catalog: {name}')
    for name in sorted(set(entries) - repositories):
        errors.append(f'catalog image absent from GHCR repositories: {name}')
    for name in sorted(expected - set(entries)):
        errors.append(f'official source image absent from catalog: {name}')
    for name, entry in sorted(entries.items()):
        versions = entry['versions']
        if not versions:
            errors.append(f'{name}: catalog contains no versions')
            continue
        if set(versions[-1]['platforms']) != SUPPORTED_PLATFORMS:
            errors.append(f'{name}: current version must support amd64 and arm64')
        for version in versions:
            reference = version['image']
            if reference.split('@')[0].rsplit('/', 1)[-1] != name:
                errors.append(f'{name}: catalog version names another repository: {reference}')
            actual = published.get(reference)
            if actual is None:
                errors.append(f'{name}: referenced GHCR manifest unreadable: {reference}')
            elif set(version['platforms']) != set(actual):
                errors.append(f'{name}: catalog platforms {sorted(version["platforms"])} differ from GHCR {sorted(actual)}: {reference}')
    return errors


def _json(*argv):
    result = subprocess.run(argv, check=True, capture_output=True, text=True, timeout=60)
    return json.loads(result.stdout)


def repositories(registry: str) -> set[str]:
    host, owner, prefix = registry.split('/', 2)
    if host != 'ghcr.io':
        raise ValueError('this audit inventories GHCR through the GitHub Packages API')
    pages = _json('gh', 'api', '--paginate', '--slurp', f'/orgs/{owner}/packages?package_type=container&per_page=100')
    return {package['name'][len(prefix) + 1:] for page in pages for package in page
            if package['name'].startswith(prefix + '/')} - INFRASTRUCTURE


@functools.cache
def platforms(reference: str) -> frozenset[str]:
    document = _json('oras', 'manifest', 'fetch', reference)
    repository = reference.split('@')[0]
    if 'manifests' in document:
        found = set()
        for child in document['manifests']:
            declared = child.get('platform', {})
            if declared.get('os') == 'unknown':  # OCI attestations are not workloads.
                continue
            actual = platforms(repository + '@' + child['digest'])
            if declared and actual != {declared.get('os', '') + '/' + declared.get('architecture', '')}:
                raise ValueError(f'{reference}: index platform differs from child config')
            found.update(actual)
        return frozenset(found)
    config = _json('oras', 'blob', 'fetch', '--output', '-', repository + '@' + document['config']['digest'])
    return frozenset({config['os'] + '/' + config['architecture']})


def audit(document: Mapping, registry: str) -> list[str]:
    inventory = repositories(registry)
    published = {}
    for entry in document['entries'].values():
        for version in entry['versions']:
            reference = version['image']
            if not reference.startswith(registry + '/') or '@sha256:' not in reference:
                raise ValueError(f'catalog reference is not pinned in {registry}: {reference}')
            published[reference] = platforms(reference)
    return coverage_errors(document, inventory, published, set(descriptions()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog', required=True, type=Path)
    parser.add_argument('--registry', required=True)
    args = parser.parse_args()
    errors = audit(json.loads(args.catalog.read_text()), args.registry)
    if errors:
        raise SystemExit('\n'.join(errors))
    print('GHCR and catalog agree on every workload image and platform')


if __name__ == '__main__':
    main()
