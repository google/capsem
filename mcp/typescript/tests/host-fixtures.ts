/** Static gateway data for the host-tools MCP tests. */

export const sandbox = {
  available_actions: ['pause', 'stop', 'fork', 'delete'], id: 'vm-1', name: 'demo', pid: 42,
  profile_id: 'code', status: 'Running',
};
export const provision = {
  available_actions: ['pause', 'stop', 'fork', 'delete'], id: 'vm-1', name: 'demo',
  profile_id: 'code', status: 'Running',
};
export const customProfile = {
  availability: {web: true, shell: true, mobile: false},
  default_rule_count: 0, description: 'Custom profile', id: 'co-work', mcp_server_count: 0,
  name: 'Co-work', plugin_count: 0, rule_count: 0, source: 'builtin',
  update_semantics: {
    new_sessions: 'use_current_profile_catalog', existing_vms: 'pinned_until_recreate',
    upgrade_action: 'recreate_vm',
  },
};

/** Canned JSON replies keyed by `METHOD /path`. */
export const routeFixtures: Record<string, object> = {
  'POST /vms/vm-1/start': provision,
  'POST /vms/vm-1/stop': {persistent: false, success: true},
  'POST /vms/vm-1/pause': {success: true},
  'POST /vms/vm-1/resume': provision,
  'DELETE /vms/vm-1/delete': {success: true},
  'POST /vms/vm-1/fork': {id: 'fork-1', name: 'copy', size_bytes: 12},
  'POST /vms/vm-1/save': {name: 'saved', success: true},
  'POST /purge': {ephemeral_purged: 1, persistent_purged: 0, purged: 1},
  'GET /vms/vm-1/files/list': {entries: []},
  'GET /vms/vm-1/logs': {logs: 'booted'},
  'GET /panics': {panics: []},
  'GET /triage': {host: {errors: [], panics: [], slow_ops: []}, rank: [], session: {}, since: '5m'},
  'GET /vms/vm-1/timeline': {events: []},
  'GET /vms/vm-1/history': {commands: [], has_more: false, total: 0},
  'GET /vms/vm-1/stats/summary': {
    allowed_requests: 1, denied_requests: 0, total_estimated_cost: 0,
    total_input_tokens: 0, total_output_tokens: 0, total_requests: 1,
    total_thinking_tokens: 0, total_tool_calls: 0,
  },
  'GET /vms/vm-1/stats/detail': {
    audit_events: [], body_blobs: {}, credential_events: [], dns_events: [], file_events: [],
    http_events: [], interactions: {bodies: [], items: []}, model_events: [], model_stats: [],
    process_events: [], tool_events: [],
  },
  'GET /vms/vm-1/snapshots/list': {snapshots: [], total: 0},
  'GET /vms/vm-1/snapshots/status': {
    auto_count: 0, manual_available: 0, manual_count: 0, snapshots: [], total: 0,
  },
  'GET /vms/vm-1/changes': {changes: [], checkpoint: 'cp-1', has_more: false, total: 0},
  'GET /vms/vm-1/container': {image: 'docker://busybox:latest', state: 'running'},
  'POST /vms/vm-1/exposures': {
    access: 'loopback_tcp', id: '49152', host_port: 49152, guest_port: 8080, target: 'container',
  },
  'GET /vms/vm-1/exposures': {
    owner_generation: '7',
    exposures: [{
      access: 'loopback_tcp', id: '49152', host_port: 49152, guest_port: 8080, target: 'container',
    }],
  },
  'DELETE /vms/vm-1/exposures/49152': {success: true},
  'POST /vms/vm-1/exposures/49152/preview-session': {
    url: 'http://49152.localhost:19223/_capsem/bootstrap', bootstrap_token: 'bootstrap-secret',
    expires_in_seconds: 30,
  },
};
