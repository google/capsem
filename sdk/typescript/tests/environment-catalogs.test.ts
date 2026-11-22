import {expect, it} from 'vitest';
import {Hypervisor} from '../src/index.js';
import {gateway} from './gateway.js';

it('reads global catalog skills for the exact selected source, version, and digest', async () => {
  const catalog = {reference: 'registry.example/skills:stable', digest: `sha256:${'0'.repeat(64)}`, snapshot: 'listed-snapshot'};
  const selection = {id: 'review', version: '1.0.0+preview', catalog_reference: catalog.reference, resolved_digest: catalog.digest};
  const metadata = {id: selection.id, version: selection.version, name: 'Review', publisher: 'Fixture', provenance: 'Fixture catalog',
    license: 'Apache-2.0', compatibility: {min_runtime_contract: 1}, status: 'active'};
  const files = {'SKILL.md': 'review fixture\n', 'references/notes.md': 'notes\n'};
  const detail = {selection, metadata, files};
  let payload: unknown = {catalog, skills: [metadata], next_cursor: 'next'};
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const page = await hv.skills.list({search: 'review', after: 'page', limit: 1});
      expect(page.next_cursor).toBe('next');
      expect(received[0]?.url).toBe('/skills/list?search=review&after=page&limit=1');
      payload = detail;
      expect((await hv.skills.get('review', page.skills[0]!.version, page.catalog)).files).toEqual(files);
      expect(received[1]?.url).toBe('/skills/review/info?version=1.0.0%2Bpreview&snapshot=listed-snapshot');
      for (const invalid of [
        {...detail, selection: {...selection, id: 'other'}},
        {...detail, selection: {...selection, version: '1'}},
        {...detail, metadata: {...metadata, id: 'other'}},
        {...detail, metadata: {...metadata, version: '1'}},
        {...detail, selection: {...selection, catalog_reference: 'registry.example/other:stable'}},
        {...detail, selection: {...selection, resolved_digest: `sha256:${'1'.repeat(64)}`}},
      ]) {
        payload = invalid;
        await expect(hv.skills.get('review', selection.version, page.catalog)).rejects.toThrow('Catalog entry identity changed');
      }
    } finally { hv.close(); }
  });
});

it('reads typed published rules from the listed catalog snapshot', async () => {
  const catalog = {reference: 'registry.example/rules:stable', digest: `sha256:${'0'.repeat(64)}`, snapshot: 'listed-snapshot'};
  const selection = {id: 'review', version: '1.0.0', catalog_reference: catalog.reference, resolved_digest: catalog.digest};
  const metadata = {id: selection.id, version: selection.version, name: 'Review', publisher: 'Fixture', provenance: 'Fixture catalog',
    license: 'Apache-2.0', compatibility: {min_runtime_contract: 1}, status: 'active'};
  const definition = {kind: 'detection', rule: {name: 'Review', match: 'http.host == {{host}}', level: 'high'}};
  const parameters = {host: {type: 'text', default: 'api.example', max_bytes: 70}};
  let payload: unknown = {catalog, rules: [{metadata, kind: 'detection'}], next_cursor: 'next'};
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const page = await hv.rules.list({search: 'review', after: 'first', limit: 12});
      expect(page.rules[0]?.kind).toBe('detection');
      expect(received[0]?.url).toBe('/rules/list?search=review&after=first&limit=12');
      const detail = {selection, metadata, definition, parameters};
      payload = detail;
      expect(await hv.rules.get('review', '1.0.0', page.catalog)).toMatchObject(detail);
      expect(received[1]?.url).toBe('/rules/review/info?version=1.0.0&snapshot=listed-snapshot');
      payload = {...detail, selection: {...selection, catalog_reference: 'registry.example/other:stable'}};
      await expect(hv.rules.get('review', '1.0.0', page.catalog)).rejects.toThrow('Catalog entry identity changed');
    } finally { hv.close(); }
  });
});

it('reads published MCP requirements and rejects detail from a different digest', async () => {
  const digest = `sha256:${'0'.repeat(64)}`;
  const catalog = {reference: `registry.example/mcp@${digest}`, digest, snapshot: 'listed-snapshot'};
  const selection = {id: 'review', version: '1.0.0', catalog_reference: catalog.reference, resolved_digest: digest};
  const metadata = {id: selection.id, version: selection.version, name: 'Review', publisher: 'Fixture', provenance: 'Fixture catalog',
    license: 'Apache-2.0', compatibility: {min_runtime_contract: 1}, status: 'active'};
  const definition = {kind: 'sandbox', executable: 'review', args: ['--stdio'],
    secret_variables: {TOKEN: {description: 'Workspace key', required: true}}};
  const detail = {selection, metadata, definition, published_tools: [{name: 'review_file'}]};
  let payload: unknown = {catalog, mcps: [{metadata, kind: 'sandbox'}]};
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const page = await hv.mcps.list({search: 'review', limit: 1});
      expect(page.mcps[0]?.kind).toBe('sandbox');
      expect(received[0]?.url).toBe('/mcps/list?search=review&limit=1');
      payload = detail;
      expect(await hv.mcps.get('review', '1.0.0', page.catalog)).toMatchObject(detail);
      expect(received[1]?.url).toBe('/mcps/review/info?version=1.0.0&snapshot=listed-snapshot');
      payload = {...detail, selection: {...selection, resolved_digest: `sha256:${'1'.repeat(64)}`}};
      await expect(hv.mcps.get('review', '1.0.0', page.catalog)).rejects.toThrow('Catalog entry identity changed');
    } finally { hv.close(); }
  });
});
