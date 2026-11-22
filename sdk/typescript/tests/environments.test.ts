import {expect, it} from 'vitest';
import {EnvironmentArchiveAction, Hypervisor, HttpError, RequestApplicationState, RequestDecision, RequestState, type EnvironmentCreateRequest, type CallOptions} from '../src/index.js';
import {gateway} from './gateway.js';

const requestId = '00000000-0000-4000-8000-000000000001';
const environmentId = '00000000-0000-4000-8000-000000000002';
const input: EnvironmentCreateRequest = {request_id: requestId, specification: {name: 'development'}, reason: 'prepare blueprint'};
const pending = {request_id: requestId, environment_id: environmentId, decision: 'approved', application: 'pending', result: null, application_error: null};
const current = {document: {identity: {schema_version: 1, id: environmentId, revision_id: requestId}, specification: {name: 'development'}}, archived: false, timestamp_unix_ms: 1, actor: 'operator', reason: 'approved', request_id: requestId};

it('lists selected skill pins and rejects detail with substituted metadata', async () => {
  const revision = {environment_id: environmentId, revision_id: requestId};
  const selection = {id: 'review', version: '1.0.0', catalog_reference: 'registry.example/skills:stable', resolved_digest: `sha256:${'0'.repeat(64)}`};
  const metadata = {id: 'review', version: '1.0.0', name: 'Review', publisher: 'Fixture', provenance: 'Fixture catalog',
    license: 'Apache-2.0', compatibility: {min_runtime_contract: 1}, status: 'active'};
  const files = {'SKILL.md': 'review fixture\n', 'references/notes.md': 'notes\n'};
  const detail = {revision, selection, metadata, files, discovery_fresh: false};
  let payload: unknown = {revision, discovery_fresh: false, skills: [{selection, metadata, availability: 'available'}]};
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      expect((await hv.environments.skills(revision, {search: 'fixture', limit: 12})).skills[0]?.selection).toEqual(selection);
      const query = new URL(received[0]!.url!, url).searchParams;
      expect(query.get('revision_id')).toBe(requestId);
      expect(query.get('search')).toBe('fixture');
      expect(query.get('limit')).toBe('12');
      payload = detail;
      expect((await hv.environments.skill(revision, 'review')).files).toEqual(files);
      expect(received[1]?.url).toBe(`/environments/${environmentId}/skills/review/info?revision_id=${requestId}`);
      for (const invalid of [
        {...detail, selection: {...selection, id: 'other'}},
        {...detail, metadata: {...metadata, id: 'other'}},
        {...detail, metadata: {...metadata, version: '2.0.0'}},
      ]) {
        payload = invalid;
        await expect(hv.environments.skill(revision, 'review')).rejects.toThrow('skill identity changed');
      }
    } finally { hv.close(); }
  });
});

it('correlates applied VM skills with the requested owner and revision', async () => {
  const revision = {environment_id: environmentId, revision_id: requestId};
  const selection = {id: 'review', version: '1.0.0', catalog_reference: 'registry.example/skills:stable', resolved_digest: `sha256:${'0'.repeat(64)}`};
  const metadata = {id: 'review', version: '1.0.0', name: 'Review', publisher: 'Fixture', provenance: 'Fixture catalog',
    license: 'Apache-2.0', compatibility: {min_runtime_contract: 1}, status: 'active'};
  const files = {'SKILL.md': 'applied fixture\n'};
  const scope = {vm_id: 'vm-1', owner_generation: requestId};
  const page = {...scope, revision, skills: [], discovery_fresh: true};
  const detail = {revision, selection, metadata, files, discovery_fresh: true};
  let payload: unknown = page;
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const vm = hv.vm({id: 'vm-1'});
      expect(await vm.skills({revisionId: requestId, limit: 1})).toEqual(page);
      const query = new URL(received[0]!.url!, url);
      expect(query.pathname).toBe('/vms/vm-1/skills/list');
      expect(query.searchParams.get('revision_id')).toBe(requestId);
      expect(query.searchParams.get('limit')).toBe('1');
      payload = {...page, revision: null};
      await expect(vm.skills({revisionId: requestId})).rejects.toThrow('revision identity changed');
      expect((await vm.skills()).revision).toBeNull();
      payload = {...page, vm_id: 'vm-2'};
      await expect(vm.skills()).rejects.toThrow('revision identity changed');

      payload = {...scope, skill: detail};
      expect((await vm.skill('review', {revisionId: requestId})).skill.files).toEqual(files);
      expect(received.at(-1)?.url).toBe(`/vms/vm-1/skills/review/info?revision_id=${requestId}`);
      expect((await vm.skill('review')).owner_generation).toBe(requestId);
      await expect(vm.skill('review', {revisionId: environmentId})).rejects.toThrow('skill identity changed');
      for (const invalid of [
        {...scope, vm_id: 'vm-2', skill: detail},
        {...scope, skill: {...detail, selection: {...selection, id: 'other'}}},
        {...scope, skill: {...detail, metadata: {...metadata, id: 'other'}}},
        {...scope, skill: {...detail, metadata: {...metadata, version: '2.0.0'}}},
      ]) {
        payload = invalid;
        await expect(vm.skill('review')).rejects.toThrow('skill identity changed');
      }
    } finally { hv.close(); }
  });
});

it('checks the VM and server identity of live MCP inventory', async () => {
  const scope = {vm_id: 'vm-1', owner_generation: requestId};
  const inputSchema = {type: 'object', properties: {query: {type: 'string'}}};
  let payload: unknown = {...scope, servers: []};
  await gateway((_request, reply) => reply.end(JSON.stringify(payload)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const vm = hv.vm({id: 'vm-1'});
      expect((await vm.mcps()).owner_generation).toBe(requestId);
      expect(received.at(-1)?.url).toBe('/vms/vm-1/mcps');
      payload = {...scope, vm_id: 'vm-2', servers: []};
      await expect(vm.mcps()).rejects.toThrow('inventory identity changed');
      payload = {...scope, server_id: 'library', tools: [{
        namespaced_name: 'library__search', original_name: 'search',
        input_schema: inputSchema, definition_digest: 'observed',
      }]};
      expect((await vm.mcpTools('library')).tools[0]?.input_schema).toEqual(inputSchema);
      expect(received.at(-1)?.url).toBe('/vms/vm-1/mcps/library/tools');
      await expect(vm.mcpTools('other')).rejects.toThrow('inventory identity changed');
    } finally { hv.close(); }
  });
});

it('runs a command from an exact saved revision and refuses inline variables', async () => {
  const reference = {environment_id: environmentId, revision_id: requestId};
  const output = {stdout: {encoding: 'utf8', data: 'pinned'}, stderr: {encoding: 'utf8', data: ''}, exit_code: 0};
  await gateway((_request, reply) => reply.end(JSON.stringify(output)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      expect((await hv.run('printf test', {environment: reference})).exit_code).toBe(0);
      await expect(hv.run('printf test', {environment: reference, env: {}})).rejects.toThrow('inline variables');
      await hv.run('printf test');
      expect(received.map(request => request.url)).toEqual(['/run', '/run']);
      expect(JSON.parse(received[0]?.body.toString() ?? '').environment).toEqual(reference);
      expect(JSON.parse(received[1]?.body.toString() ?? '')).not.toHaveProperty('environment');
    } finally { hv.close(); }
  });
});

it('archives an exact revision and continues to expose its history', async () => {
  const base = {environment_id: environmentId, revision_id: requestId};
  const archive = {action: EnvironmentArchiveAction.ARCHIVE, request_id: requestId, base_revision_id: requestId, reason: 'retire blueprint'};
  const route = `/environments/${environmentId}`;
  const history = {summary: {environment: base, actor: 'operator', reason: 'approved', replayable: true, timestamp_unix_ms: 1},
    availability: 'complete', canonical_toml: 'name = "development"', purged_variables: []};
  await gateway((request, reply) => {
    if (request.method === 'POST') reply.end(JSON.stringify({...pending, application: 'applied', result: base}));
    else if (request.url === '/environments/list') reply.end(JSON.stringify({environments: [{environment: base, archived: true}]}));
    else if (request.url === `${route}/info`) reply.end(JSON.stringify({...current, archived: true}));
    else reply.end(JSON.stringify(history));
  }, async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      expect((await hv.environments.change(environmentId, archive)).result).toEqual(base);
      expect((await hv.environments.list()).environments).toEqual([{environment: base, archived: true}]);
      expect((await hv.environments.info(environmentId)).archived).toBe(true);
      expect(await hv.environments.revision(base)).toEqual(history);
      await expect(hv.environments.revision({...base, revision_id: environmentId})).rejects.toThrow('identity changed');
      await expect(hv.environments.revision({...base, environment_id: requestId})).rejects.toThrow('identity changed');
      expect(received[0]?.url).toBe(`${route}/changes`);
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toEqual(archive);
    } finally { hv.close(); }
  });
});

it('preserves approval versus application and explicitly reconciles the same request ID', async () => {
  await gateway((request, response) => response.end(JSON.stringify(request.method === 'POST' ? pending : current)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const created = await hv.environments.create(input);
      expect(created.request_id).toBe(requestId);
      expect(created.decision).toBe(RequestState.APPROVED);
      expect(created.application).toBe(RequestApplicationState.PENDING);
      expect(created.result).toBeNull();
      const inspected = await hv.environments.info(environmentId);
      expect(inspected.document.identity.id).toBe(environmentId);
      expect(inspected.document.specification.name).toBe('development');
      expect(inspected.actor).toBe('operator');
      expect(await hv.environments.create(input)).toEqual(created);
      expect(received.map(request => [request.method, request.url])).toEqual([
        ['POST', '/environments/create'], ['GET', `/environments/${environmentId}/info`], ['POST', '/environments/create'],
      ]);
      for (const request of received) expect(request.headers.authorization).toBe('Bearer token');
      for (const request of received.filter(request => request.method === 'POST')) {
        expect(JSON.parse(request.body.toString())).toEqual(input);
      }
      hv.close();
      await expect(hv.environments.create(input)).rejects.toThrow('closed');
      expect(received).toHaveLength(3);
    } finally { hv.close(); }
  });
});

it('retains conflicts without mutation replay or implicit refresh', async () => {
  await gateway((_request, response) => response.writeHead(409).end('changed intent'), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const failure = hv.environments.create(input);
      await expect(failure).rejects.toBeInstanceOf(HttpError);
      await expect(failure).rejects.toMatchObject({status: 409});
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('refuses a substituted request identity', async () => {
  await gateway((_request, response) => response.end(JSON.stringify({...pending, request_id: environmentId})), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      await expect(hv.environments.create(input)).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('refuses a substituted environment identity', async () => {
  await gateway((_request, response) => response.end(JSON.stringify(current)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      await expect(hv.environments.info(requestId)).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('reads review provenance and refuses a changed request identity', async () => {
  const summary = {
    request_id: requestId, kind: 'environment_change', environment_id: environmentId, actor: 'operator',
    reason: 'test review', timestamp_unix_ms: 1, expires_at_unix_ms: 1000, decision: 'pending', application: 'pending',
  };
  const detail = {
    summary, observed_revision: '0123456789ab',
    proposed: {availability: 'complete', purged_variables: [], specification: {name: 'development', variables: {EDITOR: 'emacs'}}},
    targets: [],
  };
  await gateway((request, response) => response.end(JSON.stringify(
    request.method === 'POST' ? {...pending, decision: 'denied'} :
      request.url?.startsWith('/requests/list') ? {requests: [summary], next_cursor: 9} : detail,
  )), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      expect(await hv.requests.list({after: 7, limit: 1, pendingOnly: true})).toMatchObject({next_cursor: 9});
      expect(await hv.requests.info(requestId)).toEqual(detail);
      await expect(hv.requests.info(environmentId)).rejects.toThrow('identity changed');
      const decision = {decision: RequestDecision.DENY, observed_revision: detail.observed_revision, reason: 'keep the original configuration'};
      expect((await hv.requests.resolve(requestId, decision)).decision).toBe(RequestState.DENIED);
      expect(received.map(request => request.url)).toEqual([
        '/requests/list?after=7&limit=1&pending_only=true', `/requests/${requestId}/info`, `/requests/${environmentId}/info`,
        `/requests/${requestId}/resolve`,
      ]);
      expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).toEqual(decision);
    } finally { hv.close(); }
  });
});

it.each(['create', 'change'] as const)('retains the %s budget, deadlines and cancellation without replay', async mutation => {
  await gateway((_request, response) => {setTimeout(() => response.end(JSON.stringify(pending)), 150);}, async (url, received) => {
    const hv = new Hypervisor(url, 'token', {timeoutMs: 30});
    try {
      const invoke = (options: CallOptions = {}) => mutation === 'create' ? hv.environments.create(input, options) : hv.environments.change(environmentId, {...input, base_revision_id: requestId}, options);
      expect((await invoke()).request_id).toBe(requestId);
      await expect(invoke({timeoutMs: 30})).rejects.toMatchObject({name: 'TimeoutError'});
      const controller = new AbortController();
      const rejected = expect(invoke({signal: controller.signal})).rejects.toMatchObject({name: 'AbortError'});
      setTimeout(() => controller.abort(), 30);
      await rejected;
      expect(received.map(request => request.url)).toEqual(Array<string>(3).fill(mutation === 'create' ? '/environments/create' : `/environments/${environmentId}/changes`));
    } finally { hv.close(); }
  });
});


it.each([
  {response: pending, valid: true},
  {response: {...pending, request_id: environmentId}, valid: false},
  {response: {...pending, environment_id: requestId}, valid: false},
])('preserves change base and rejects substituted identities ($valid)', async ({response, valid}) => {
  await gateway((_request, reply) => reply.end(JSON.stringify(response)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const base = '00000000-0000-4000-8000-000000000003';
      const request = {...input, base_revision_id: base};
      const result = hv.environments.change(environmentId, request);
      if (valid) expect((await result).application).toBe(RequestApplicationState.PENDING);
      else await expect(result).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
      expect(received[0]?.url).toBe(`/environments/${environmentId}/changes`);
      expect(received[0]?.headers.authorization).toBe('Bearer token');
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toEqual(request);
    } finally { hv.close(); }
  });
});

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
