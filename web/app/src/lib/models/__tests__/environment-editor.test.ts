import { describe, expect, it } from 'vitest';
import { EnvironmentImageUpdatePolicy } from '@capsem/sdk';
import { editableFields, environmentChanges, environmentDraft, reviewedDraft } from '../environment-editor';

const digest = `sha256:${'a'.repeat(64)}`;
const pin = { id: 'coding', version: '1.0.0', catalog_reference: 'registry.example/skills:stable', resolved_digest: digest };
const specification: Parameters<typeof environmentDraft>[0] = {
  name: 'workspace', description: 'Saved settings', image: { reference: 'redis', resolved_digest: digest },
  repositories: [{ url: 'https://git.example/repo', ref: 'main', resolved_commit: 'a'.repeat(40), destination: 'src' }],
  skills: { selected: [pin] }, access: { catalog: { selected: [pin] }, parameters: { coding: { count: 3 } } },
  detection: { catalog: { selected: [pin] } }, mcps: { catalog: { selected: [pin] } },
  startup: { script: 'printf first\nprintf second\n', timeout_seconds: 17 },
  variables: { EDITOR: 'vim', MODE: 'test' },
  secrets: { example: { binding_id: '00000000-0000-4000-8000-000000000002' } },
};

describe('environment editing', () => {
  it('takes up saved selectors and keeps configured sections separate from resolution facts', () => {
    const draft = reviewedDraft(editableFields(specification));
    expect(draft.image).toEqual({ reference: 'redis', update_policy: EnvironmentImageUpdatePolicy.MANUAL });
    expect(draft.repositories).toEqual([{ url: 'https://git.example/repo', ref: 'main', destination: 'src' }]);
    for (const selected of [draft.skills, draft.access?.catalog, draft.detection?.catalog, draft.mcps?.catalog]) {
      expect(selected?.selected).toEqual([{ id: pin.id, version: pin.version }]);
    }
    expect(draft.access?.parameters).toEqual(specification.access?.parameters);
    expect(draft.startup).toEqual(specification.startup);
    expect(draft.variables).toEqual(specification.variables);
    expect(draft.secrets).toEqual(specification.secrets);
    expect(JSON.stringify(draft)).not.toContain(digest);
  });

  it('reviews the complete changed field and ignores JSON key ordering', () => {
    const fields = editableFields(specification);
    const before = reviewedDraft(fields);
    fields.sections.variables = '{"MODE":"test","EDITOR":"vim"}';
    expect(environmentChanges(before, reviewedDraft(fields))).toEqual([]);
    fields.script += 'printf third\n';
    expect(environmentChanges(before, reviewedDraft(fields))).toEqual([{
      label: 'Startup', before: 'Timeout 17 s\n\nprintf first\nprintf second\n',
      after: 'Timeout 17 s\n\nprintf first\nprintf second\nprintf third\n',
    }]);
  });

  it('rejects malformed configuration and caller-supplied resolution data before review', () => {
    const fields = editableFields(specification);
    fields.sections.secrets = '{invalid JSON';
    expect(() => reviewedDraft(fields)).toThrow('Secret bindings contains invalid JSON.');
    fields.sections.secrets = '{}';
    fields.sections.repositories = JSON.stringify(specification.repositories);
    expect(() => reviewedDraft(fields)).toThrow('Check the configuration for Repositories.');
    fields.image = '';
    expect(() => reviewedDraft(fields)).toThrow('Choose an image before adding a startup script.');
  });
});
