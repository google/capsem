import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Mock fetch globally before importing api.
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

// Mock WebSocket globally.
const mockWsSend = vi.fn();
const mockWsClose = vi.fn();
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
  });

  // ---- init / healthCheck ----

  describe('init', () => {
    it('returns connected=true when health and token succeed', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok123' }));

      const result = await api.init();
      expect(result.connected).toBe(true);
      expect(result.reachable).toBe(true);
      expect(result.version).toBe('1.0.0');
      expect(api.isConnected()).toBe(true);
    });

    it('returns connected=false when health fails', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({}, 500));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(false);
      expect(api.isConnected()).toBe(false);
    });

    it('returns connected=false, reachable=true when token fails', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({}, 401));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(true);
    });

    it('returns connected=false on network error', async () => {
      mockFetch.mockRejectedValueOnce(new Error('network'));

      const result = await api.init();
      expect(result.connected).toBe(false);
      expect(result.reachable).toBe(false);
    });
  });

  describe('healthCheck', () => {
    it('returns true on 200', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ ok: true }));
      expect(await api.healthCheck()).toBe(true);
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
  });

  // ---- VM lifecycle ----

  describe('VM lifecycle', () => {
    beforeEach(async () => {
      // Connect first.
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('provisionVm sends POST /vms/create', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ id: 'vm-1' }));
      const result = await api.provisionVm({ ram_mb: 2048, cpus: 2, persistent: false });
      expect(result.id).toBe('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/create');
      expect(call[1].method).toBe('POST');
    });

    it('runVm sends POST /run', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ id: 'vm-2' }));
      const result = await api.runVm({ ram_mb: 4096, cpus: 4, persistent: true });
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

    it('persistVm sends POST', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.persistVm('vm-1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/save');
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
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
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

    it('inspectQuery sends POST /vms/{id}/inspect', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ columns: ['n'], rows: [{ n: 1 }] }));
      const result = await api.inspectQuery('vm-1', 'SELECT 1 as n');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/inspect');
      expect(result.columns).toEqual(['n']);
    });
  });

  // ---- Settings ----

  describe('settings', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
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

  // ---- MCP config (via settings) ----

  describe('MCP config via settings', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('setMcpServerEnabled calls saveSettings with correct key', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ tree: [], issues: [] }));
      await api.setMcpServerEnabled('my-server', true);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      const body = JSON.parse(call[1].body);
      expect(body['mcp.servers.my-server.enabled']).toBe(true);
    });

    it('addMcpServer calls saveSettings with url, enabled, headers, token', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ tree: [], issues: [] }));
      await api.addMcpServer('srv', 'http://x', { 'X-Key': 'val' }, 'tok123');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      const body = JSON.parse(call[1].body);
      expect(body['mcp.servers.srv.url']).toBe('http://x');
      expect(body['mcp.servers.srv.enabled']).toBe(true);
      expect(body['mcp.servers.srv.headers']).toEqual({ 'X-Key': 'val' });
      expect(body['mcp.servers.srv.bearer_token']).toBe('tok123');
    });

    it('removeMcpServer sends null for the server key', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({ tree: [], issues: [] }));
      await api.removeMcpServer('old-srv');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      const body = JSON.parse(call[1].body);
      expect(body['mcp.servers.old-srv']).toBeNull();
    });

    it('does not expose retired MCP policy mutators', () => {
      expect('getMcpPolicy' in api).toBe(false);
      expect('setMcpGlobalPolicy' in api).toBe(false);
      expect('setMcpDefaultPermission' in api).toBe(false);
      expect('setMcpToolPermission' in api).toBe(false);
    });
  });

  // ---- Profiles ----

  describe('profiles', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('listProfiles sends GET /profiles/list', async () => {
      const profiles = {
        profiles: [
          {
            id: 'default',
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
          id: 'default',
          name: 'Default',
          description: 'Built-in Capsem developer profile.',
          source: 'effective',
          rule_count: 3,
          default_rule_count: 2,
          plugin_count: 1,
          mcp_server_count: 0,
        },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(info));
      const result = await api.getProfileInfo('default');
      expect(result).toEqual(info);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/info');
    });

    it('validateProfile sends POST /profiles/{profile_id}/validate', async () => {
      const response = { valid: true, profile_id: 'default' };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.validateProfile('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/validate');
      expect(call[1].method).toBe('POST');
    });

    it('profile mutation helpers use explicit profile routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.createProfile({});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/create');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('POST');

      await api.editProfile('default', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/edit');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('PATCH');

      await api.deleteProfile('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/delete');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('DELETE');

      await api.cloneProfile('default', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/clone');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('POST');
    });

    it('profile skill helpers use profile-scoped routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.getProfileSkillsInfo('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/skills/info');

      await api.listProfileSkills('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/skills/list');

      await api.addProfileSkill('default', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/skills/add');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('POST');

      await api.editProfileSkill('default', 'build', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/skills/build/edit');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('PATCH');

      await api.deleteProfileSkill('default', 'build');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/skills/build/delete');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('DELETE');
    });

    it('profile credential helpers use profile-scoped routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.getProfileCredentialsInfo('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/info');

      await api.getProfileCredentialsStatus('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/status');

      await api.listProfileCredentials('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/list');

      await api.reloadProfileCredentials('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/reload');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('POST');

      await api.getProfileCredentialInfo('default', 'openai');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/openai/info');

      await api.deleteProfileCredential('default', 'openai');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/credentials/openai/delete');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('DELETE');
    });

    it('profile asset, plugin, and mcp info helpers use profile-scoped routes', async () => {
      mockFetch.mockReturnValue(jsonResponse({ ok: true }));

      await api.getProfileAssetsInfo('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/assets/info');

      await api.editProfileAssets('default', {});
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/assets/edit');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].method).toBe('PATCH');

      await api.getProfilePluginsInfo('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/plugins/info');

      await api.getProfileMcpInfo('default');
      expect(mockFetch.mock.calls[mockFetch.mock.calls.length - 1][0]).toContain('/profiles/default/mcp/info');
    });
  });

  // ---- Enforcement rules ----

  describe('enforcement rules', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('listEnforcementRules sends GET /profiles/{profile_id}/enforcement/rules/list', async () => {
      const response = {
        profile_id: 'default',
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
      const result = await api.listEnforcementRules('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/enforcement/rules/list');
    });

    it('getEnforcementInfo sends GET /profiles/{profile_id}/enforcement/info', async () => {
      const response = {
        profile_id: 'default',
        rule_count: 8,
        default_rule_count: 7,
        custom_rule_count: 1,
        detection_rule_count: 2,
        plugin_rule_count: 1,
        corp_locked_rule_count: 0,
        source_counts: { builtin_default: 7, profile: 1 },
        action_counts: { allow: 7, block: 1 },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getEnforcementInfo('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/enforcement/info');
    });
  });

  describe('detection rules', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('listDetectionRules sends GET /profiles/{profile_id}/detection/rules/list', async () => {
      const response = {
        profile_id: 'default',
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
      const result = await api.listDetectionRules('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/detection/rules/list');
    });

    it('getDetectionInfo sends GET /profiles/{profile_id}/detection/info', async () => {
      const response = {
        profile_id: 'default',
        rule_count: 2,
        default_rule_count: 1,
        custom_rule_count: 1,
        detection_rule_count: 2,
        plugin_rule_count: 0,
        corp_locked_rule_count: 0,
        source_counts: { builtin_default: 1, profile: 1 },
        action_counts: { allow: 2 },
      };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getDetectionInfo('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/detection/info');
    });
  });

  // ---- Plugins ----

  describe('plugins', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('listPlugins sends GET /profiles/{profile_id}/plugins/list', async () => {
      const plugins = {
        scope: { kind: 'profile', profile_id: 'default' },
        plugins: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(plugins));
      const result = await api.listPlugins('default');
      expect(result).toEqual(plugins);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/plugins/list');
    });

    it('updatePlugin sends PATCH /profiles/{profile_id}/plugins/{plugin_id}/edit', async () => {
      const plugin = {
        id: 'dummy_pre_eicar',
        config: { mode: 'block', detection_level: 'high' },
        default_config: { mode: 'rewrite', detection_level: 'informational' },
        overridden: true,
        scope: { kind: 'profile', profile_id: 'strict' },
        description: 'debug plugin',
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
  });

  // ---- MCP runtime ----

  describe('MCP runtime', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('getMcpServers sends GET /profiles/{profile_id}/mcp/servers/list', async () => {
      const servers = [{ name: 'srv', url: 'http://x', enabled: true }];
      mockFetch.mockReturnValueOnce(jsonResponse(servers));
      const result = await api.getMcpServers('default');
      expect(result).toEqual(servers);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/mcp/servers/list');
    });

    it('getMcpServers returns [] when disconnected', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init(); // disconnect
      const result = await api.getMcpServers('default');
      expect(result).toEqual([]);
    });

    it('getMcpTools sends GET /profiles/{profile_id}/mcp/servers/{server_id}/tools/list', async () => {
      // Re-connect after the disconnected test above.
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      const tools = [{ namespaced_name: 'bash', server_name: 'system' }];
      mockFetch.mockReturnValueOnce(jsonResponse(tools));
      const result = await api.getMcpTools('default', 'system');
      expect(result).toEqual(tools);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/mcp/servers/system/tools/list');
    });

    it('refreshMcpTools sends POST /profiles/{profile_id}/mcp/servers/{server_id}/refresh', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.refreshMcpTools('default', 'my-server');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/mcp/servers/my-server/refresh');
    });

    it('approveMcpTool sends PATCH /profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/edit', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.approveMcpTool('default', 'local', 'bash');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/mcp/servers/local/tools/bash/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({ approved: true });
    });

    it('callMcpTool sends POST /profiles/{profile_id}/mcp/servers/{server_id}/tools/{tool_id}/call', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({ result: 'ok' }));
      const result = await api.callMcpTool('default', 'local', 'bash', { command: 'ls' });
      expect(result).toEqual({ result: 'ok' });
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/mcp/servers/local/tools/bash/call');
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
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        service: 'running',
        gateway_version: '1.0.0',
        vm_count: 1,
        vms: [{ id: 'vm-1', name: null, status: 'Running', persistent: false }],
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
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
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
  });

  // ---- App actions ----

  describe('checkForAppUpdate', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
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
    it('sends POST /profiles/default/reload by default', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.reloadProfile();
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/reload');
      expect(call[1].method).toBe('POST');
    });
  });

  describe('profile assets', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();
    });

    it('getAssetsStatus sends GET /profiles/{profile_id}/assets/status', async () => {
      const response = { ready: true, assets: [], missing: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getAssetsStatus('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/assets/status');
    });

    it('ensureAssets sends POST /profiles/{profile_id}/assets/ensure', async () => {
      const response = { ready: true, ensured: true, downloaded: 0, assets: [], missing: [] };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.ensureAssets('default');
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/profiles/default/assets/ensure');
      expect(call[1].method).toBe('POST');
    });
  });

  describe('getImages', () => {
    it('sends GET /images', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({ images: [{ name: 'default' }] }));
      const result = await api.getImages();
      expect(result.images).toHaveLength(1);
    });
  });
});
