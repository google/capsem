import { describe, expect, it } from 'vitest';
import { reviewedDraft, editableFields, environmentChanges } from '../models/environment-editor';
import { currentCatalogSelection, stageCatalogSelection } from '../models/environment-catalog';

const selection = {
  id: 'review', version: '2.0.0', catalog_reference: 'registry.example/team/skills:stable',
  resolved_digest: `sha256:${'a'.repeat(64)}`,
};
const expected = {
  id: 'review', version: '2.0.0', expected_catalog: {
    reference: selection.catalog_reference, digest: selection.resolved_digest,
  },
};

describe('catalog selections in an editable draft', () => {
  it('changes only the selected skill and puts the inspected snapshot in review', () => {
    const fields = editableFields();
    fields.name = 'test-environment';
    fields.sections.skills = JSON.stringify({ selected: [{ id: 'other', version: '1' }] });
    const before = reviewedDraft(fields);
    fields.sections.skills = stageCatalogSelection(fields.sections.skills, 'skills', selection);
    const after = reviewedDraft(fields);
    expect(after.skills?.selected).toEqual([{ id: 'other', version: '1' }, expected]);
    expect(environmentChanges(before, after)).toMatchObject([{ label: 'Skills' }]);
    expect(environmentChanges(before, after)[0]?.after).toContain(selection.resolved_digest);
  });

  it('preserves an unchanged saved selector even if the published digest moved', () => {
    const baseline = JSON.stringify({ selected: [{ id: 'review', version: '2.0.0' }] });
    expect(stageCatalogSelection(baseline, 'skills', selection)).toBe(baseline);
    expect(currentCatalogSelection(baseline, 'skills', selection.id)?.expected_catalog).toBeUndefined();
    const staged = stageCatalogSelection('{}', 'skills', selection);
    expect(stageCatalogSelection(staged, 'skills', { ...selection, resolved_digest: `sha256:${'b'.repeat(64)}` }))
      .toContain('b'.repeat(64));
  });

  it('uses the replacement rule version defaults and preserves other rules and parameters', () => {
    const input = JSON.stringify({
      custom: { stop: { name: 'Stop', action: 'block', match: 'http.host == "invalid.example"' } },
      parameters: { review: { host: 'old.example' }, other: { host: 'keep.example' } },
      catalog: { selected: [{ id: 'review', version: '1' }, { id: 'other', version: '3' }] },
    });
    const result = JSON.parse(stageCatalogSelection(input, 'access', selection));
    expect(result.catalog.selected).toEqual([expected, { id: 'other', version: '3' }]);
    expect(result.parameters).toEqual({ other: { host: 'keep.example' } });
    expect(result.custom).toEqual(JSON.parse(input).custom);
  });

  it('leaves invalid section text for the operator to repair', () => {
    expect(() => stageCatalogSelection('{"selected":42}', 'skills', selection)).toThrow('Check the configuration for Skills');
    expect(currentCatalogSelection('{', 'skills', selection.id)).toBeUndefined();
  });
});
