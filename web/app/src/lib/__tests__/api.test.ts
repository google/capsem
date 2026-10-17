import { updateStatusFixture } from './sdk-catalog-fixtures';
import { AssetFileState, ValidationStatus, type AssetStatus } from '@capsem/sdk';
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

function jsonResponse(body: unknown, status = 200): Promise<Response> {
  return Promise.resolve(new Response(JSON.stringify(body), { status }));
}

function assetStatusFixture(): AssetStatus {
  return {
    ready: true,
    downloading: false,
    current_arch: 'arm64',
    asset_version: '2026.1001.1',
    assets: [{
      kind: 'rootfs',
      name: 'rootfs.squashfs',
      path: '/home/u/.capsem/assets/arm64/rootfs.squashfs',
      status: AssetFileState.PRESENT,
      expected_hash: 'b'.repeat(64),
    }],
    errors: [],
    manifest: { origin: 'installed', path: '/home/u/.capsem/assets/manifest.json', validation_status: ValidationStatus.VALID },
  };
}

function textResponse(text: string, status = 200): Promise<Response> {
  return Promise.resolve(new Response(text, { status }));
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
      expect(new Headers(mockFetch.mock.calls[2][1].headers).get('Authorization')).toBe('Bearer tok123');
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

    it('debugSnapshot reads status, assets, corp, and update routes', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }))
        .mockReturnValueOnce(jsonResponse(assetStatusFixture()))
        .mockReturnValueOnce(jsonResponse({ installed: true, source: { content_hash: 'blake3:test' } }))
        .mockReturnValueOnce(jsonResponse(updateStatusFixture()));

      const snapshot = await api.debugSnapshot() as Record<string, unknown>;

      expect(snapshot.connected).toBe(true);
      expect((snapshot.status as Record<string, unknown>).service).toBe('running');
      expect((snapshot.assets_status as AssetStatus).current_arch).toBe('arm64');
      expect((snapshot.corp_info as Record<string, unknown>).installed).toBe(true);
      expect((snapshot.update_status as Record<string, any>).binary.update_available).toBe(true);
      const paths = mockFetch.mock.calls.slice(-4).map(call => call[0]);
      expect(paths[0]).toContain('/status');
      expect(paths[1]).toBe(api.getBaseUrl() + '/assets/status');
      expect(paths[2]).toContain('/corp/info');
      expect(paths[3]).toContain('/update/status');
    });
  });

  describe('getVmInfo', () => {
    it('reads per-session info with live telemetry counters', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok123' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({
        id: 'session 1',
        name: 'Demo',
        pid: 123,
        status: 'Running',
        persistent: true,
        can_resume: false,
        available_actions: ['pause', 'stop', 'fork'],
        total_input_tokens: 12,
        total_thinking_tokens: 3,
        total_output_tokens: 7,
        total_tool_calls: 2,
        total_estimated_cost: 0.001,
      }));

      const info = await api.getVmInfo('session 1');

      expect(mockFetch.mock.calls.at(-1)?.[0]).toContain('/vms/session%201/info');
      expect(info.total_input_tokens).toBe(12);
      expect(info.total_thinking_tokens).toBe(3);
      expect(info.total_output_tokens).toBe(7);
      expect(info.total_tool_calls).toBe(2);
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
      mockFetch.mockReturnValueOnce(jsonResponse({
        stdout: { encoding: 'utf8', data: 'hello' },
        stderr: { encoding: 'utf8', data: '' },
        exit_code: 0,
        truncated: false,
      }));
      const result = await api.execCommand('vm-1', 'echo hello');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm-1/exec');
      expect(result.stdout.data).toBe('hello');
      expect(result.exit_code).toBe(0);
    });

    it('getVmStatsDetail sends GET /vms/{id}/stats/detail', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        interactions: { items: [], bodies: [] },
        model_stats: [{ provider: 'google', model: 'fixture-model', call_count: 1, duration_ms: 25, input_tokens: 12, output_tokens: 7, estimated_cost_usd: 0.001 }],
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

    it('getVmStatsSummary reads the compact active-session counters', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        total_requests: 3,
        allowed_requests: 2,
        denied_requests: 1,
        total_input_tokens: 12,
        total_thinking_tokens: 3,
        total_output_tokens: 7,
        total_tool_calls: 2,
        total_estimated_cost: 0.001,
      }));

      const result = await api.getVmStatsSummary('vm 1');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/vms/vm%201/stats/summary');
      expect(result.total_thinking_tokens).toBe(3);
      expect(result.total_tool_calls).toBe(2);
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

    it('VM detection and enforcement helpers use per-session runtime routes', async () => {
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

  // ---- MCP config ----

  describe('MCP config', () => {
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

  describe('runtime ledger', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('uses service-wide security, enforcement, and detection ledger routes', async () => {
      mockFetch.mockImplementation(() => jsonResponse({ total: 0, sessions: [] }));

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

    it('listPlugins sends GET /plugins/list', async () => {
      const plugins = {
        plugins: [
          {
            id: 'credential_broker',
            name: 'Credential Broker',
            config: { mode: 'rewrite', detection_level: 'informational' },
            default_config: { mode: 'rewrite', detection_level: 'informational' },
            overridden: false,
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
                path: '/plugins/credential_broker/credentials/info',
              },
              {
                id: 'credential_broker_credentials_reload',
                label: 'Retry Credential Store',
                kind: 'credential_broker',
                path: '/plugins/credential_broker/credentials/reload',
              },
            ],
          },
        ],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(plugins));
      const result = await api.listPlugins();
      expect(result).toEqual(plugins);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/plugins/list');
    });

    it('updatePlugin sends PATCH /plugins/{plugin_id}/edit', async () => {
      const plugin = {
        id: 'dummy_pre_eicar',
        name: 'Dummy Preprocess EICAR',
        config: { mode: 'block', detection_level: 'high' },
        default_config: { mode: 'rewrite', detection_level: 'informational' },
        overridden: true,
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
      const result = await api.updatePlugin('dummy_pre_eicar', {
        mode: 'block',
        detection_level: 'high',
      });
      expect(result).toEqual(plugin);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/plugins/dummy_pre_eicar/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({
        mode: 'block',
        detection_level: 'high',
      });
    });

    it('does not expose retired global plugin authoring helpers', () => {
      expect(api.listPlugins.length).toBe(0);
      expect(api.updatePlugin.length).toBe(2);
    });

    it('getCredentialBrokerInfo sends GET /plugins/credential_broker/credentials/info', async () => {
      const detail = {
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
          enabled: true,
          vm_grants: [],
          fork_default: 'inherit',
        },
        corp_constraints: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(detail));
      const result = await api.getCredentialBrokerInfo();
      expect(result).toEqual(detail);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/plugins/credential_broker/credentials/info');
    });

    it('reloadCredentialBrokerStore sends POST /plugins/credential_broker/credentials/reload', async () => {
      const detail = {
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
          enabled: true,
          vm_grants: [],
          fork_default: 'inherit',
        },
        corp_constraints: [],
      };
      mockFetch.mockReturnValueOnce(jsonResponse(detail));
      const result = await api.reloadCredentialBrokerStore();
      expect(result).toEqual(detail);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/plugins/credential_broker/credentials/reload');
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

    it('getMcpServers sends GET /mcp/servers/list', async () => {
      const servers = [{ name: 'srv', url: 'http://x', enabled: true }];
      mockFetch.mockReturnValueOnce(jsonResponse(servers));
      const result = await api.getMcpServers();
      expect(result).toEqual(servers);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/servers/list');
    });

    it('getMcpServers returns [] when disconnected', async () => {
      mockFetch.mockRejectedValueOnce(new Error('fail'));
      await api.init(); // disconnect
      const result = await api.getMcpServers();
      expect(result).toEqual([]);
    });

    it('getMcpDefaultPermission sends GET /mcp/default/info', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      const permission = { action: 'allow', source: 'default', rule_id: 'default.mcp' };
      mockFetch.mockReturnValueOnce(jsonResponse(permission));
      const result = await api.getMcpDefaultPermission();
      expect(result).toEqual(permission);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/default/info');
    });

    it('getMcpTools sends GET /mcp/servers/{server_id}/tools/list', async () => {
      // Re-connect after the disconnected test above.
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      const tools = [{ namespaced_name: 'bash', server_name: 'system' }];
      mockFetch.mockReturnValueOnce(jsonResponse(tools));
      const result = await api.getMcpTools('system');
      expect(result).toEqual(tools);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/servers/system/tools/list');
    });

    it('refreshMcpTools sends POST /mcp/servers/{server_id}/refresh', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.refreshMcpTools('my-server');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/servers/my-server/refresh');
      expect(call[1].method).toBe('POST');
    });

    it('updateMcpToolPermission sends PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.updateMcpToolPermission('local', 'bash', 'ask');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/servers/local/tools/bash/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({ action: 'ask' });
    });

    it('updateMcpDefaultPermission sends PATCH /mcp/default/edit', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse(null));
      await api.updateMcpDefaultPermission('block');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/default/edit');
      expect(call[1].method).toBe('PATCH');
      expect(JSON.parse(call[1].body)).toEqual({ action: 'block' });
    });

    it('callMcpTool sends POST /mcp/servers/{server_id}/tools/{tool_id}/call', async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();

      mockFetch.mockReturnValueOnce(jsonResponse({ result: 'ok' }));
      const result = await api.callMcpTool('local', 'bash', { command: 'ls' });
      expect(result).toEqual({ result: 'ok' });
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/mcp/servers/local/tools/bash/call');
      expect(JSON.parse(call[1].body)).toEqual({ command: 'ls' });
    });
  });

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

  describe('update actions', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('checks for updates through the service-owned POST route', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        status: 'succeeded',
        command: { program: 'capsem', args: ['update', '--check'] },
        exit_code: 0,
      }));

      const result = await api.checkForUpdates();

      expect(result.command.args).toEqual(['update', '--check']);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/update/check');
      expect(call[1].method).toBe('POST');
      expect(call[1].headers.Authorization).toBe('Bearer tok');
      expect(JSON.parse(String(call[1].body))).toEqual({});
    });

    it('applies the complete update transaction through one confirmed body', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        status: 'planned',
        command: { program: 'capsem', args: ['update', '--yes'] },
      }));

      await api.applyUpdate({ confirmed: true });

      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/update/apply');
      expect(call[1].method).toBe('POST');
      expect(JSON.parse(String(call[1].body))).toEqual({
        confirmed: true,
      });
    });

    it('plans the complete update transaction without confirmation only through dry run', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse({
        status: 'planned',
        command: { program: 'capsem', args: ['update', '--yes'] },
      }));

      await api.applyUpdate({ dry_run: true });

      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(JSON.parse(String(call[1].body))).toEqual({
        dry_run: true,
      });
    });
  });

  describe('getUpdateStatus', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('reads the unified update status route', async () => {
      mockFetch.mockReturnValueOnce(jsonResponse(updateStatusFixture()));

      const result = await api.getUpdateStatus();

      expect(result.binary.update_available).toBe(true);
      expect(result.assets.latest).toBe('assets-2');
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toContain('/update/status');
      expect(new Headers(call[1].headers).get('Authorization')).toBe('Bearer tok');
    });
  });

  describe('getCapsemStatus', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('reads manifest, metadata, assets, and update state from one canonical route', async () => {
      const payload = {
        version: '1.5.1',
        service: 'running',
        manifest: { channel: 'stable', packages: [] },
        manifest_metadata: {
          schema: 'capsem.manifest_metadata.v1',
          manifest_url: 'https://release.capsem.org/assets/stable/manifest.json',
        },
        assets: assetStatusFixture(),
        corp: { installed: false },
        updates: updateStatusFixture(),
      };
      const callsBefore = mockFetch.mock.calls.length;
      mockFetch.mockReturnValueOnce(jsonResponse(payload));

      const result = await api.getCapsemStatus();

      expect(result).toEqual(payload);
      expect(mockFetch.mock.calls).toHaveLength(callsBefore + 1);
      expect(mockFetch.mock.calls.at(-1)?.[0]).toContain('/system/status');
    });
  });

  // ---- Misc ----

  describe('getBaseUrl', () => {
    it('returns default base URL', () => {
      expect(api.getBaseUrl()).toBe('http://127.0.0.1:19222');
    });
  });

  describe('assets', () => {
    beforeEach(async () => {
      mockFetch
        .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
        .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
        .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
      await api.init();
    });

    it('getAssetsStatus sends GET /assets/status', async () => {
      const response = assetStatusFixture();
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.getAssetsStatus();
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/assets/status');
    });

    it('ensureAssets sends POST /assets/ensure', async () => {
      const response = { ...assetStatusFixture(), ready: false, downloading: true, started: true };
      mockFetch.mockReturnValueOnce(jsonResponse(response));
      const result = await api.ensureAssets();
      expect(result).toEqual(response);
      const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
      expect(call[0]).toBe(api.getBaseUrl() + '/assets/ensure');
      expect(JSON.parse(call[1].body)).toEqual({});
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

      mockFetch.mockReturnValueOnce(jsonResponse({ images: [{ name: 'code', description: 'Tools', architectures: ['amd64'], cached: 'unknown', image: null }] }));
      const result = await api.getImages();
      expect(result.images).toHaveLength(1);
    });
  });
});
