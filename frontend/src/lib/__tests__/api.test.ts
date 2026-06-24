import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Mock fetch globally before importing api.
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

// Mock WebSocket globally.
const mockWsSend = vi.fn();
const mockWsClose = vi.fn();
const mockWsUrls: string[] = [];
let wsOnMessage: ((ev: { data: string }) => void) | null = null;
let wsOnOpen: (() => void) | null = null;
let wsOnClose: (() => void) | null = null;

class MockWebSocket {
  static OPEN = 1;
  readyState = 1;
  binaryType = '';
  url: string;
  send = mockWsSend;
  close = mockWsClose;
  addEventListener = vi.fn();
  removeEventListener = vi.fn();

  constructor(url: string) {
    this.url = url;
    mockWsUrls.push(url);
  }

  set onmessage(fn: any) { wsOnMessage = fn; }
  set onopen(fn: any) { wsOnOpen = fn; }
  set onclose(fn: any) { wsOnClose = fn; }
}

vi.stubGlobal('WebSocket', MockWebSocket);
(MockWebSocket as any).OPEN = 1;

// Import after mocks are in place.
const api = await import('../api');

function jsonResponse(body: unknown, status = 200) {
  return Promise.resolve({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
  });
}

function textResponse(text: string, status = 200) {
  return Promise.resolve({
    ok: status >= 200 && status < 300,
    status,
    json: () => Promise.resolve(JSON.parse(text)),
    text: () => Promise.resolve(text),
  });
}

describe('api', () => {
  beforeEach(() => {
    mockFetch.mockReset();
    mockWsSend.mockReset();
    mockWsClose.mockReset();
    mockWsUrls.length = 0;
    wsOnMessage = null;
    wsOnOpen = null;
    wsOnClose = null;
    vi.useRealTimers();
  });

  // ---- init / healthCheck ----

  describe('init', () => {
    it('returns connected=true when health token and service status succeed', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok123' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));

      const result = await api.init();
      expect(result.connected).toBe(true);
      expect(result.reachable).toBe(true);
      expect(result.version).toBe('1.0.0');
      expect(result.reason).toBe('ok');
      expect(api.isConnected()).toBe(true);
      expect(mockFetch.mock.calls[2][0]).toContain('/status');
      expect(mockFetch.mock.calls[2][1].headers.Authorization).toBe('Bearer tok123');
    });

    it('returns connected=false when gateway is reachable but service status is unavailable', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok123' }))
        .mockReturnValueOnce(jsonResponse({ service: 'unavailable', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(true);
      expect(result.version).toBe('1.0.0');
      expect(result.reason).toBe('service_unavailable');
      expect(api.isConnected()).toBe(false);
    });

    it('returns connected=false when health fails', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({}, 500));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(false);
      expect(result.reason).toBe('offline');
      expect(api.isConnected()).toBe(false);
    });

    it('returns connected=false, reachable=true when token fails', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({}, 401));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(true);
      expect(result.reason).toBe('auth');
    });

    it('returns connected=false on network error', async () => {
      mockFetch.mockRejectedValueOnce(new Error('network'));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(false);
      expect(result.reason).toBe('offline');
    });
  });

  describe('healthCheck', () => {
    it('returns true when gateway health and service status are running', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      expect(await api.healthCheck()).toBe(true);
    });

    it('returns false when gateway health is ok but service status is unavailable', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true }))
        .mockReturnValueOnce(jsonResponse({ service: 'unavailable', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      expect(await api.healthCheck()).toBe(false);
      expect(api.isConnected()).toBe(false);
    });

    it('returns false on 500', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({}, 500));
      expect(await api.healthCheck()).toBe(false);
    });

    it('returns false on network error', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      expect(await api.healthCheck()).toBe(false);
    });
  });

  // ---- Status ----

  describe('getStatus', () => {
    it('returns empty status when disconnected', async () => {
      // Force disconnected state.
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init();

      const status = await api.getStatus();
      expect(status.service).toBe('offline');
      expect(status.vms).toEqual([]);
    });

    it('debugSnapshot reads status, profiles status, and corp info routes', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }))
        .mockReturnValueOnce(jsonResponse({ source: 'built_in', profile_count: 1, ready_count: 1, profiles: [] }))
        .mockReturnValueOnce(jsonResponse({ installed: true, source: { content_hash: 'blake3:test' } }));

      const snapshot = await api.debugSnapshot() as Record<string, unknown>;

      expect(snapshot.connected).toBe(true);
      expect((snapshot.status as Record<string, unknown>).service).toBe('running');
      expect((snapshot.profiles_status as Record<string, unknown>).profile_count).toBe(1);
      expect((snapshot.corp_info as Record<string, unknown>).installed).toBe(true);
      const paths = mockFetch.mock.calls.slice(-3).map(call => call[0]);
      expect(paths[0]).toContain('/status');
      expect(paths[1]).toContain('/profiles/status');
      expect(paths[2]).toContain('/corp/info');
    });
  });

  // ---- VM lifecycle ----

  describe('VM lifecycle', () => {
    beforeEach(async () => {
      // Connect first.
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('provisionVm sends POST /vms/create', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ id: 'vm-1' }));
      const result = await api.provisionVm({
        profile_id: 'code',
        name: 'code-dev',
        ram_mb: 2048,
        cpus: 2,
        persistent: true,
      });
      expect(result.id).toBe('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/create');
      expect(call[1].method).toBe('POST');
      expect(JSON.parse(call[1].body).profile_id).toBe('code');
    });

    it('refreshes a rotated gateway token and retries VM creation once', async () => {
      mockFetch
        .mockReturnValueOnce(textResponse('{"error":"unauthorized"}', 401))
        .mockReturnValueOnce(jsonResponse({ token: 'fresh-token' }))
        .mockReturnValueOnce(jsonResponse({ id: 'vm-fresh' }));

      const result = await api.provisionVm({
        profile_id: 'code',
        name: 'code-dev',
        ram_mb: 2048,
        cpus: 2,
        persistent: true,
      });

      expect(result.id).toBe('vm-fresh');
      const createCalls = mockFetch.mock.calls.filter(call => String(call[0]).includes('/vms/create'));
      expect(createCalls).toHaveLength(2);
      expect(createCalls[0][1].headers.Authorization).toBe('Bearer tok');
      expect(createCalls[1][1].headers.Authorization).toBe('Bearer fresh-token');
      expect(mockFetch.mock.calls.some(call => String(call[0]).endsWith('/token'))).toBe(true);
    });

    it('runVm sends POST /run', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ id: 'vm-2' }));
      const result = await api.runVm({
        profile_id: 'code',
        ram_mb: 4096,
        cpus: 4,
        persistent: true,
      });
      expect(result.id).toBe('vm-2');
    });

    it('stopVm sends POST /vms/{id}/stop', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.stopVm('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/stop');
    });

    it('deleteVm sends DELETE /vms/{id}/delete', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.deleteVm('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/delete');
      expect(call[1].method).toBe('DELETE');
    });

    it('suspendVm sends POST', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.suspendVm('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/pause');
    });

    it('resumeVm sends POST', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.resumeVm('my-vm');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/my-vm/resume');
    });

    it('forkVm sends POST with body', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ name: 'fork-1', size_bytes: 1024 }));
      const result = await api.forkVm('vm-1', { name: 'fork-1' });
      expect(result.name).toBe('fork-1');
      expect(result.size_bytes).toBe(1024);
    });
  });

  // ---- VM inspection ----

  describe('VM inspection', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('execCommand sends POST /vms/{id}/exec', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ stdout: 'hello', stderr: '', exit_code: 0 }));
      const result = await api.execCommand('vm-1', 'echo hello');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/exec');
      expect(result.stdout).toBe('hello');
      expect(result.exit_code).toBe(0);
    });

    it('readFile sends POST /vms/{id}/files/read', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ content: 'file contents' }));
      const result = await api.readFile('vm-1', '/etc/hosts');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/files/read');
      expect(result.content).toBe('file contents');
    });

    it('writeFile sends POST /vms/{id}/files/write', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.writeFile('vm-1', '/tmp/test', 'data');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/files/write');
      const body = JSON.parse(call[1].body);
      expect(body.path).toBe('/tmp/test');
      expect(body.content).toBe('data');
    });

    it('getVmStatsDetail sends GET /vms/{id}/stats/detail', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        model_stats: [{ provider: 'google', call_count: 1 }],
        model_events: [],
        tool_events: [],
        http_events: [],
        dns_events: [],
        file_events: [],
        process_events: [],
        audit_events: [],
        credential_events: [],
        body_blobs: {},
      }));
      const result = await api.getVmStatsDetail('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/stats/detail');
      expect(result.model_stats[0].provider).toBe('google');
    });

    it('getVmSecurityLatest sends GET /vms/{id}/security/latest with limit', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse([
        {
          timestamp_unix_ms: 1700000000000,
          event_id: 'abc123abc123',
          event_type: 'http.request',
          rule_id: 'profiles.rules.default_http',
          rule_action: 'allow',
          detection_level: 'none',
          rule_json: '{}',
          event_json: '{}',
          trace_id: null,
        },
      ]));
      const result = await api.getVmSecurityLatest('vm-1', 25);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/security/latest?limit=25');
      expect(result[0].event_id).toBe('abc123abc123');
    });

    it('getVmSecurityStatus sends GET /vms/{id}/security/status', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        total: 1,
        by_action: [{ rule_action: 'block', count: 1 }],
        by_event_type: [{ event_type: 'dns.query', count: 1 }],
        by_level: [{ detection_level: 'high', count: 1 }],
        by_rule: [{
          rule_id: 'corp.rules.block_dns',
          rule_action: 'block',
          detection_level: 'high',
          count: 1,
          latest_event_id: 'abc123abc123',
          latest_timestamp_unix_ms: 1700000000000,
        }],
      }));
      const result = await api.getVmSecurityStatus('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/security/status');
      expect(result.by_rule[0].rule_id).toBe('corp.rules.block_dns');
    });

    it('VM detection and enforcement helpers use profile-scoped runtime routes', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse([]))
        .mockReturnValueOnce(jsonResponse({ total: 0, by_action: [], by_event_type: [], by_level: [], by_rule: [] }))
        .mockReturnValueOnce(jsonResponse([]))
        .mockReturnValueOnce(jsonResponse({ total: 0, by_action: [], by_event_type: [], by_level: [], by_rule: [] }));

      await api.getVmDetectionLatest('vm-1', 5);
      await api.getVmDetectionStatus('vm-1');
      await api.getVmEnforcementLatest('vm-1', 7);
      await api.getVmEnforcementStatus('vm-1');

      const paths = mockFetch.mock.calls.slice(-4).map(call => call[0]);
      expect(paths[0]).toContain('/vms/vm-1/detection/latest?limit=5');
      expect(paths[1]).toContain('/vms/vm-1/detection/status');
      expect(paths[2]).toContain('/vms/vm-1/enforcement/latest?limit=7');
      expect(paths[3]).toContain('/vms/vm-1/enforcement/status');
    });
  });

  // ---- Settings ----

  describe('settings', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('getSettings sends GET /settings/info', async () => {
      const mockResp = { tree: [], issues: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(mockResp));
      const result = await api.getSettings();
      expect(result).toEqual(mockResp);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/settings/info');
      expect(call[1].method).toBeUndefined(); // GET (no method override)
    });

    it('saveSettings sends PATCH /settings/edit with changes', async () => {
      const changes = { 'vm.resources.cpu_count': 8 };
      const mockResp = { tree: [], issues: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(mockResp));
      const result = await api.saveSettings(changes);
      expect(result).toEqual(mockResp);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/settings/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual(changes);
    });

  });

  // ---- MCP profile config ----

  describe('MCP profile config', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('does not expose retired MCP policy or settings mutators', () => {
      expect('updateMcpServer' in api).toBe(false);
      expect('upsertMcpServer' in api).toBe(false);
      expect('deleteMcpServer' in api).toBe(false);
      expect('getMcpPolicy' in api).toBe(false);
      expect('setMcpGlobalPolicy' in api).toBe(false);
      expect('setMcpDefaultPermission' in api).toBe(false);
      expect('setMcpToolPermission' in api).toBe(false);
      expect('setMcpServerEnabled' in api).toBe(false);
      expect('addMcpServer' in api).toBe(false);
      expect('removeMcpServer' in api).toBe(false);
    });
  });

  // ---- Profiles ----

  describe('profiles', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('listProfiles sends GET /profiles/list', async () => {
      const profiles = {
        profiles: [
          {
            id: 'code',
            name: 'Default',
            description: 'Built-in Capsem developer profile.',
            source: 'effective',
            rule_count: 3,
            default_rule_count: 2,
            plugin_count: 1,
            mcp_server_count: 0,
          },
        ],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(profiles));
      const result = await api.listProfiles();
      expect(result).toEqual(profiles);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/list');
    });

    it('getProfileInfo sends GET /profiles/{profile_id}/info', async () => {
      const info = {
        profile: {
          id: 'code',
          name: 'Default',
          description: 'Built-in Capsem developer profile.',
          source: 'effective',
          rule_count: 3,
          default_rule_count: 2,
          plugin_count: 1,
          mcp_server_count: 0,
        },
        obom: {
          profile_id: 'code',
          current_arch: 'arm64',
          scope: 'base_image',
          format: 'cyclonedx-obom.v1',
          name: 'obom.cdx.json',
          url: 'file:///tmp/capsem/obom.cdx.json',
          hash: `blake3:${'1'.repeat(64)}`,
          size: 123,
          generator: 'cdxgen',
          generator_version: '11.0.0',
          rootfs_hash: `blake3:${'2'.repeat(64)}`,
          route: '/profiles/code/obom',
        },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(info));
      const result = await api.getProfileInfo('code');
      expect(result).toEqual(info);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/info');
    });

    it('getProfileObom sends GET /profiles/{profile_id}/obom', async () => {
      const response = {
        profile_id: 'code',
        current_arch: 'arm64',
        obom: {
          profile_id: 'code',
          current_arch: 'arm64',
          scope: 'base_image',
          format: 'cyclonedx-obom.v1',
          name: 'obom.cdx.json',
          url: 'file:///tmp/capsem/obom.cdx.json',
          hash: `blake3:${'1'.repeat(64)}`,
          size: 123,
          generator: 'cdxgen',
          generator_version: '11.0.0',
          rootfs_hash: `blake3:${'2'.repeat(64)}`,
          route: '/profiles/code/obom',
        },
        document: { bomFormat: 'CycloneDX' },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getProfileObom('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/obom');
    });

    it('validateProfile sends POST /profiles/{profile_id}/validate', async () => {
      const response = { valid: true, profile_id: 'code' };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.validateProfile('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/validate');
      expect(call[1].method).toBe('POST');
    });

    it('profile skill helpers use profile-scoped routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.getProfileSkillsInfo('code');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/skills/info');

      await api.listProfileSkills('code');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/skills/list');

      await api.addProfileSkill('code', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/skills/add');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('POST');

      await api.editProfileSkill('code', 'build', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/skills/build/edit');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('PATCH');

      await api.deleteProfileSkill('code', 'build');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/skills/build/delete');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('DELETE');
    });

    it('profile asset, plugin, and mcp info helpers use profile-scoped routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.getProfileAssetsInfo('code');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/assets/info');

      await api.getProfilePluginsInfo('code');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/plugins/info');

      await api.getProfileMcpInfo('code');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/code/mcp/info');
    });
  });

  // ---- Enforcement rules ----

  describe('enforcement rules', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('listEnforcementRules sends GET /profiles/{profile_id}/enforcement/rules/list', async () => {
      const response = {
        profile_id: 'code',
        rules: [
          {
            rule_id: 'profiles.rules.default_http_requests',
            source: 'builtin_default',
            provider: 'profiles',
            namespace: 'profiles',
            rule_key: 'default_http_requests',
            default_rule: true,
            name: 'default_http_requests',
            action: 'ask',
            match: 'http.request.exists()',
            priority: 0,
            corp_locked: false,
          },
        ],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.listEnforcementRules('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/enforcement/rules/list');
    });

    it('getEnforcementInfo sends GET /profiles/{profile_id}/enforcement/info', async () => {
      const response = {
        profile_id: 'code',
        rule_count: 8,
        default_rule_count: 7,
        custom_rule_count: 1,
        detection_rule_count: 2,
        corp_locked_rule_count: 0,
        source_counts: { builtin_default: 7, profile: 1 },
        action_counts: { allow: 7, block: 1 },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getEnforcementInfo('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/enforcement/info');
    });
  });

  describe('detection rules', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('listDetectionRules sends GET /profiles/{profile_id}/detection/rules/list', async () => {
      const response = {
        profile_id: 'code',
        rules: [
          {
            rule_id: 'profiles.rules.skill_loaded',
            source: 'profile',
            provider: 'profiles',
            namespace: 'profiles',
            rule_key: 'skill_loaded',
            default_rule: false,
            name: 'skill_loaded',
            action: 'allow',
            match: 'file.read.path.contains("skills/")',
            detection_level: 'informational',
            priority: 10,
            corp_locked: false,
          },
        ],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.listDetectionRules('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/detection/rules/list');
    });

    it('getDetectionInfo sends GET /profiles/{profile_id}/detection/info', async () => {
      const response = {
        profile_id: 'code',
        rule_count: 2,
        default_rule_count: 1,
        custom_rule_count: 1,
        detection_rule_count: 2,
        corp_locked_rule_count: 0,
        source_counts: { builtin_default: 1, profile: 1 },
        action_counts: { allow: 2 },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getDetectionInfo('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/detection/info');
    });
  });

  describe('runtime ledger', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('uses service-wide security, enforcement, and detection ledger routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ total: 0, sessions: [] }));

      await api.getSecurityLatest();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/security/latest');

      await api.getSecurityStatus();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/security/status');

      await api.getEnforcementLatest();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/enforcement/latest');

      await api.getEnforcementStatus();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/enforcement/status');

      await api.getDetectionLatest();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/detection/latest');

      await api.getDetectionStatus();
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/detection/status');
    });
  });

  // ---- Plugins ----

  describe('plugins', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('listPlugins sends GET /profiles/{profile_id}/plugins/list', async () => {
      const plugins = {
        scope: { kind: 'profile', profile_id: 'code' },
        plugins: [
          {
            id: 'credential_broker',
            name: 'Credential Broker',
            config: { mode: 'rewrite', detection_level: 'informational' },
            default_config: { mode: 'rewrite', detection_level: 'informational' },
            overridden: false,
            scope: { kind: 'profile', profile_id: 'code' },
            description: 'captures observed credentials',
            stage: 'preprocess',
            version: '1',
            capabilities: {
              event_families: ['http', 'file', 'mcp'],
              credential_providers: ['anthropic', 'google', 'openai', 'github', 'mcp'],
              credential_sources: [
                'http.authorization',
                'http.body.oauth_token',
                'file.env',
                'mcp.auth_reference',
              ],
            },
            runtime: {
              enabled: true,
              event_count: 0,
              execution_count: 0,
              applied_count: 0,
              skipped_count: 0,
              total_duration_us: 0,
              max_duration_us: 0,
              detection_count: 0,
              block_count: 0,
              rewrite_count: 0,
              last_error: null,
              brokered_credentials: [],
            },
            detail_routes: [
              {
                id: 'credential_broker_credentials',
                label: 'Credential Broker',
                kind: 'credential_broker',
                path: '/profiles/code/plugins/credential_broker/credentials/info',
              },
              {
                id: 'credential_broker_credentials_reload',
                label: 'Retry Credential Store',
                kind: 'credential_broker',
                path: '/profiles/code/plugins/credential_broker/credentials/reload',
              },
            ],
          },
        ],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(plugins));
      const result = await api.listPlugins('code');
      expect(result).toEqual(plugins);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/plugins/list');
    });

    it('updatePlugin sends PATCH /profiles/{profile_id}/plugins/{plugin_id}/edit', async () => {
      const plugin = {
        id: 'dummy_pre_eicar',
        name: 'Dummy Preprocess EICAR',
        config: { mode: 'block', detection_level: 'high' },
        default_config: { mode: 'rewrite', detection_level: 'informational' },
        overridden: true,
        scope: { kind: 'profile', profile_id: 'strict' },
        description: 'debug plugin',
        stage: 'preprocess',
        version: '1',
        capabilities: {
          event_families: ['http', 'model', 'file', 'mcp'],
          credential_providers: [],
          credential_sources: [],
        },
        runtime: {
          enabled: true,
          event_count: 1,
          execution_count: 1,
          applied_count: 1,
          skipped_count: 0,
          total_duration_us: 25,
          max_duration_us: 25,
          detection_count: 1,
          block_count: 1,
          rewrite_count: 0,
          last_error: null,
          brokered_credentials: [],
        },
        detail_routes: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(plugin));
      const result = await api.updatePlugin('strict', 'dummy_pre_eicar', {
        mode: 'block',
        detection_level: 'high',
      });
      expect(result).toEqual(plugin);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/strict/plugins/dummy_pre_eicar/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({
        mode: 'block',
        detection_level: 'high',
      });
    });

    it('does not expose retired global plugin authoring helpers', () => {
      expect(api.listPlugins.length).toBe(1);
      expect(api.updatePlugin.length).toBe(3);
    });

    it('getCredentialBrokerInfo sends GET /profiles/{profile_id}/plugins/credential_broker/credentials/info', async () => {
      const detail = {
        scope: { kind: 'profile', profile_id: 'code' },
        plugin_id: 'credential_broker',
        store: {
          backend: 'test_disk',
          ready: true,
          status: 'ready',
          cached_count: 0,
          last_hydrated_count: 0,
          last_hydrated_unix_ms: null,
          last_error: null,
        },
        inventory: [],
        grants: {
          profile_enabled: true,
          vm_grants: [],
          fork_default: 'inherit_profile',
        },
        corp_constraints: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(detail));
      const result = await api.getCredentialBrokerInfo('code');
      expect(result).toEqual(detail);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/plugins/credential_broker/credentials/info');
    });

    it('reloadCredentialBrokerStore sends POST /profiles/{profile_id}/plugins/credential_broker/credentials/reload', async () => {
      const detail = {
        scope: { kind: 'profile', profile_id: 'code' },
        plugin_id: 'credential_broker',
        store: {
          backend: 'test_disk',
          ready: true,
          status: 'ready',
          cached_count: 1,
          last_hydrated_count: 1,
          last_hydrated_unix_ms: 1789000123456,
          last_error: null,
        },
        inventory: [],
        grants: {
          profile_enabled: true,
          vm_grants: [],
          fork_default: 'inherit_profile',
        },
        corp_constraints: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(detail));
      const result = await api.reloadCredentialBrokerStore('code');
      expect(result).toEqual(detail);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/plugins/credential_broker/credentials/reload');
      expect(call[1].method).toBe('POST');
    });
  });

  // ---- MCP runtime ----

  describe('MCP runtime', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('getMcpServers sends GET /profiles/{profile_id}/mcp/servers/list', async () => {
      const servers = [{ name: 'srv', url: 'http://x', enabled: true }];
      mockFetch.mockReturnValueOnce(jsonResponse(servers));
      const result = await api.getMcpServers('code');
      expect(result).toEqual(servers);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/servers/list');
    });

    it('getMcpServers returns [] when disconnected', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init(); // disconnect
      const result = await api.getMcpServers('code');
      expect(result).toEqual([]);
    });

    it('getMcpDefaultPermission sends GET /profiles/{profile_id}/mcp/default/info', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      const permission = { action: 'allow', source: 'default', rule_id: 'default.mcp' };
      mockFetch.mockReturnValueOnce(jsonResponse(permission));
      const result = await api.getMcpDefaultPermission('code');
      expect(result).toEqual(permission);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/default/info');
    });

    it('getMcpTools sends GET /profiles/{profile_id}/mcp/servers/{server_id}/tools/list', async () => {
      // Re-connect after the disconnected test above.
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      const tools = [{ namespaced_name: 'bash', server_name: 'system' }];
      mockFetch.mockReturnValueOnce(jsonResponse(tools));
      const result = await api.getMcpTools('code', 'system');
      expect(result).toEqual(tools);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/servers/system/tools/list');
    });

    it('refreshMcpTools sends POST /profiles/{profile_id}/mcp/servers/{server_id}/refresh', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.refreshMcpTools('code', 'my-server');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/servers/my-server/refresh');
    });

    it('updateMcpToolPermission sends PATCH /profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.updateMcpToolPermission('code', 'local', 'bash', 'ask');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/servers/local/tools/bash/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({ action: 'ask' });
    });

    it('updateMcpDefaultPermission sends PATCH /profiles/{profile_id}/mcp/default/edit', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.updateMcpDefaultPermission('code', 'block');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/default/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({ action: 'block' });
    });

    it('callMcpTool sends POST /profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({ result: 'ok' }));
      const result = await api.callMcpTool('code', 'local', 'bash', { command: 'ls' });
      expect(result).toEqual({ result: 'ok' });
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/mcp/servers/local/tools/bash/call');
    });

    it('getVmSnapshotStatus reads the snapshot route instead of session SQL', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        total: 1,
        auto_count: 1,
        manual_count: 0,
        manual_available: 12,
        snapshots: [{ checkpoint: 'cp-0', slot: 0, origin: 'auto', timestamp: 'unix:1' }],
      }));
      const result = await api.getVmSnapshotStatus('code-1');
      expect(result.total).toBe(1);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/code-1/snapshots/status');
    });

    it('listVmSnapshots reads the snapshot list route', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        total: 1,
        snapshots: [{ checkpoint: 'cp-0', slot: 0, origin: 'auto', timestamp: 'unix:1' }],
      }));
      const result = await api.listVmSnapshots('code-1');
      expect(result.snapshots[0].checkpoint).toBe('cp-0');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/code-1/snapshots/list');
    });
  });

  // ---- VM state ----

  describe('VM state', () => {
    it('vmStatus returns not created when disconnected', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init();
      const state = await api.vmStatus();
      expect(state).toBe('not created');
    });

    it('vmStatus returns running VM status when connected', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        service: 'running',
        gateway_version: '1.0.0',
        vm_count: 1,
        vms: [{ id: 'vm-1', name: 'code-dev', status: 'Running', persistent: true }],
        resource_summary: null,
      }));
      const state = await api.vmStatus();
      expect(state).toBe('running');
    });

    it('getVmState returns empty when disconnected', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init();
      const state = await api.getVmState();
      expect(state.state).toBe('not created');
      expect(state.history).toEqual([]);
      expect(state.elapsed_ms).toBe(0);
    });

    it('getVmState with id sends GET /vms/{id}/status', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        status: 'running',
        elapsed_ms: 3100,
        history: [{ from: 'booting', to: 'running', trigger: 'boot_complete', duration_ms: 3100, timestamp: '2026-01-01' }],
      }));
      const state = await api.getVmState('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/status');
      expect(state.state).toBe('running');
      expect(state.elapsed_ms).toBe(3100);
      expect(state.history).toHaveLength(1);
    });
  });

  // ---- Events (WebSocket) ----

  describe('onVmStateChanged / onDownloadProgress', () => {
    it('onVmStateChanged returns unsubscribe function', () => {
      const cb = vi.fn();
      const unsub = api.onVmStateChanged(cb);
      expect(typeof unsub).toBe('function');
      unsub();
    });

    it('onDownloadProgress returns unsubscribe function', () => {
      const cb = vi.fn();
      const unsub = api.onDownloadProgress(cb);
      expect(typeof unsub).toBe('function');
      unsub();
    });

    it('refreshes token before reconnecting events websocket after gateway restart', async () => {
      vi.useFakeTimers();
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'old-token' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
      expect(mockWsUrls.at(-1)).toContain('token=old-token');

      mockFetch.mockReturnValueOnce(jsonResponse({ token: 'new-token' }));
      wsOnClose?.();
      await vi.advanceTimersByTimeAsync(5000);

      expect(mockWsUrls.at(-1)).toContain('token=new-token');
      expect(mockFetch.mock.calls.some(call => String(call[0]).endsWith('/token'))).toBe(true);
    });
  });

  // ---- App actions ----

  describe('checkForAppUpdate', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('returns update info when available', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ version: '2.0.0', current_version: '1.0.0' }));
      const result = await api.checkForAppUpdate();
      expect(result).toEqual({ version: '2.0.0', current_version: '1.0.0' });
    });

    it('returns null on error', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      const result = await api.checkForAppUpdate();
      expect(result).toBeNull();
    });
  });

  // ---- Terminal ----

  describe('terminal', () => {
    it('getTerminalWsUrl constructs correct URL', () => {
      const url = api.getTerminalWsUrl('vm-1');
      expect(url).toContain('ws://');
      expect(url).toContain('/terminal/vm-1');
      expect(url).toContain('token=');
    });

    it('serialInput does not throw when no WebSocket', async () => {
      await api.serialInput('hello');
      // No error thrown.
    });

    it('terminalResize does not throw when no WebSocket', async () => {
      await api.terminalResize(80, 24);
      // No error thrown.
    });

    it('onTerminalSourceChanged returns unsubscribe function', async () => {
      const cb = vi.fn();
      const unsub = await api.onTerminalSourceChanged(cb);
      expect(typeof unsub).toBe('function');
      unsub();
    });
  });

  // ---- Misc ----

  describe('getBaseUrl', () => {
    it('returns default base URL', () => {
      expect(api.getBaseUrl()).toBe('http://127.0.0.1:19222');
    });
  });

  describe('reloadProfile', () => {
    it('sends POST /profiles/{profile_id}/reload', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.reloadProfile('co-work');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/co-work/reload');
      expect(call[1].method).toBe('POST');
    });
  });

  describe('profile assets', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('getAssetsStatus sends GET /profiles/{profile_id}/assets/status', async () => {
      const response = { ready: true, assets: [], missing: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getAssetsStatus('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/assets/status');
    });

    it('ensureAssets sends POST /profiles/{profile_id}/assets/ensure', async () => {
      const response = { ready: true, ensured: true, downloaded: 0, assets: [], missing: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.ensureAssets('code');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/code/assets/ensure');
      expect(call[1].method).toBe('POST');
    });
  });

  describe('getImages', () => {
    it('sends GET /images', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({ images: [{ name: 'code' }] }));
      const result = await api.getImages();
      expect(result.images).toHaveLength(1);
    });
  });
});
