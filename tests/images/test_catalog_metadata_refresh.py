"""A metadata refresh cannot build apps or replace their proven stable pins."""

from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]


def test_metadata_refresh_is_explicit_and_cannot_publish_after_a_failed_build() -> None:
    workflow = yaml.safe_load((ROOT / '.github/workflows/images.yaml').read_text())
    triggers = workflow.get('on', workflow.get(True))
    mode = triggers['workflow_dispatch']['inputs']['catalog_only']
    assert mode['type'] == 'boolean' and mode['default'] is False
    assert workflow['jobs']['base-arch']['if'] == '${{ !inputs.catalog_only }}'
    catalog = workflow['jobs']['catalog']
    assert catalog['if'] == "${{ always() && (inputs.catalog_only || needs.image.result == 'success') }}"
    downloads = [step for step in catalog['steps'] if step.get('uses', '').startswith('actions/download-artifact@')]
    assert len(downloads) == 2
    assert all(step['if'] == '${{ !inputs.catalog_only }}' for step in downloads)
    merge = next(step['run'] for step in catalog['steps'] if step.get('id') == 'merge')
    assert 'metadata refresh requires an existing catalog' in merge
    assert 'mkdir -p records' in merge
