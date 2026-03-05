// Mock data for browser-only dev mode (no Tauri backend).
// Active when window.__TAURI_INTERNALS__ is absent.
import type {
  ConfigIssue,
  QueryResult,
  ResolvedSetting,
  SessionInfo,
  SettingsNode,
  VmStateResponse,
  GuestConfigResponse,
  NetworkPolicyResponse,
} from './types';

export const isMock = typeof window !== 'undefined' && !(window as any).__TAURI_INTERNALS__;

// ---------------------------------------------------------------------------
// Static mock data
// ---------------------------------------------------------------------------

// Helper: creates a default mock setting with sensible defaults for empty fields.
function ms(overrides: Partial<ResolvedSetting> & { id: string; category: string; name: string; setting_type: ResolvedSetting['setting_type'] }): ResolvedSetting {
  return {
    description: '',
    default_value: overrides.setting_type === 'bool' ? false : overrides.setting_type === 'number' ? 0 : '',
    effective_value: overrides.setting_type === 'bool' ? false : overrides.setting_type === 'number' ? 0 : '',
    source: 'default',
    modified: null,
    corp_locked: false,
    enabled_by: null,
    enabled: true,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {} },
    ...overrides,
  };
}

let mockSettings: ResolvedSetting[] = [
  // -- AI Providers --
  ms({
    id: 'ai.anthropic.allow', category: 'AI Providers', name: 'Allow Anthropic', setting_type: 'bool',
    description: 'Enable API access to Anthropic (api.anthropic.com).',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'ai.anthropic.api_key', category: 'AI Providers', name: 'Anthropic API Key', setting_type: 'apikey',
    description: 'API key for Anthropic. Injected as ANTHROPIC_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.anthropic.domains', category: 'AI Providers', name: 'Anthropic Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.anthropic.com, *.claude.com', effective_value: '*.anthropic.com, *.claude.com',
    enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.anthropic.claude.settings_json', category: 'AI Providers', name: 'Claude Code settings.json', setting_type: 'file',
    description: 'Content for ~/.claude/settings.json.',
    default_value: { path: '/root/.claude/settings.json', content: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}' },
    effective_value: { path: '/root/.claude/settings.json', content: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}' },
    enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.anthropic.claude.state_json', category: 'AI Providers', name: 'Claude Code state (.claude.json)', setting_type: 'file',
    description: 'Content for ~/.claude.json. Skips onboarding.',
    default_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}' },
    effective_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}' },
    enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.openai.allow', category: 'AI Providers', name: 'Allow OpenAI', setting_type: 'bool',
    description: 'Enable API access to OpenAI (api.openai.com).',
    default_value: false, effective_value: false,
    corp_locked: true, source: 'corp',
  }),
  ms({
    id: 'ai.openai.api_key', category: 'AI Providers', name: 'OpenAI API Key', setting_type: 'apikey',
    description: 'API key for OpenAI. Injected as OPENAI_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.openai.allow', enabled: false,
  }),
  ms({
    id: 'ai.openai.domains', category: 'AI Providers', name: 'OpenAI Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.openai.com', effective_value: '*.openai.com',
    enabled_by: 'ai.openai.allow', enabled: false,
  }),
  ms({
    id: 'ai.google.allow', category: 'AI Providers', name: 'Allow Google AI', setting_type: 'bool',
    description: 'Enable API access to Google AI (*.googleapis.com).',
    default_value: true, effective_value: true,
  }),
  ms({
    id: 'ai.google.api_key', category: 'AI Providers', name: 'Google AI API Key', setting_type: 'apikey',
    description: 'API key for Google AI. Injected as GEMINI_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.domains', category: 'AI Providers', name: 'Google AI Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.googleapis.com', effective_value: '*.googleapis.com',
    enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.gemini.settings_json', category: 'AI Providers', name: 'Gemini settings.json', setting_type: 'file',
    description: 'Content for ~/.gemini/settings.json.',
    default_value: { path: '/root/.gemini/settings.json', content: '{"approvalMode":"yolo","general":{"enableAutoUpdate":false}}' },
    effective_value: { path: '/root/.gemini/settings.json', content: '{"approvalMode":"yolo","general":{"enableAutoUpdate":false}}' },
    enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.gemini.projects_json', category: 'AI Providers', name: 'Gemini projects.json', setting_type: 'file',
    description: 'Content for ~/.gemini/projects.json.',
    default_value: { path: '/root/.gemini/projects.json', content: '{"projects":{"/root":"root"}}' },
    effective_value: { path: '/root/.gemini/projects.json', content: '{"projects":{"/root":"root"}}' },
    enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.gemini.trusted_folders_json', category: 'AI Providers', name: 'Gemini trustedFolders.json', setting_type: 'file',
    description: 'Content for ~/.gemini/trustedFolders.json.',
    default_value: { path: '/root/.gemini/trustedFolders.json', content: '{"/root":"TRUST_FOLDER"}' },
    effective_value: { path: '/root/.gemini/trustedFolders.json', content: '{"/root":"TRUST_FOLDER"}' },
    enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.gemini.installation_id', category: 'AI Providers', name: 'Gemini installation_id', setting_type: 'file',
    description: 'Stable UUID avoids first-run prompts.',
    default_value: { path: '/root/.gemini/installation_id', content: 'capsem-sandbox-00000000-0000-0000-0000-000000000000' },
    effective_value: { path: '/root/.gemini/installation_id', content: 'capsem-sandbox-00000000-0000-0000-0000-000000000000' },
    enabled_by: 'ai.google.allow',
  }),
  // -- Search --
  ms({
    id: 'search.google.allow', category: 'Search', name: 'Allow Google Search', setting_type: 'bool',
    description: 'Enable access to Google web search.',
    default_value: true, effective_value: true,
    metadata: { domains: ['www.google.com', 'google.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'search.perplexity.allow', category: 'Search', name: 'Allow Perplexity', setting_type: 'bool',
    description: 'Enable access to Perplexity AI search.',
    default_value: false, effective_value: false,
    metadata: { domains: ['perplexity.ai', '*.perplexity.ai'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'search.firecrawl.allow', category: 'Search', name: 'Allow Firecrawl', setting_type: 'bool',
    description: 'Enable access to Firecrawl web scraping API.',
    default_value: false, effective_value: false,
    metadata: { domains: ['firecrawl.dev', 'api.firecrawl.dev'], choices: [], min: null, max: null, rules: {} },
  }),
  // -- Package Registries --
  ms({
    id: 'registry.github.allow', category: 'Package Registries', name: 'Allow GitHub', setting_type: 'bool',
    description: 'Enable access to GitHub and GitHub-hosted content.',
    default_value: true, effective_value: true,
    corp_locked: true, source: 'corp',
    metadata: { domains: ['github.com', '*.github.com', '*.githubusercontent.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.npm.allow', category: 'Package Registries', name: 'Allow npm', setting_type: 'bool',
    description: 'Enable access to the npm package registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['registry.npmjs.org', '*.npmjs.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.pypi.allow', category: 'Package Registries', name: 'Allow PyPI', setting_type: 'bool',
    description: 'Enable access to the Python Package Index.',
    default_value: true, effective_value: true,
    metadata: { domains: ['pypi.org', 'files.pythonhosted.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.crates.allow', category: 'Package Registries', name: 'Allow crates.io', setting_type: 'bool',
    description: 'Enable access to the Rust crate registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['crates.io', 'static.crates.io'], choices: [], min: null, max: null, rules: {} },
  }),
  // -- Guest Environment --
  ms({
    id: 'guest.shell.term', category: 'Guest Environment', name: 'TERM', setting_type: 'text',
    description: 'Terminal type for the guest shell.',
    default_value: 'xterm-256color', effective_value: 'xterm-256color',
  }),
  ms({
    id: 'guest.shell.home', category: 'Guest Environment', name: 'HOME', setting_type: 'text',
    description: 'Home directory for the guest shell.',
    default_value: '/root', effective_value: '/root',
  }),
  ms({
    id: 'guest.shell.path', category: 'Guest Environment', name: 'PATH', setting_type: 'text',
    description: 'Executable search path for the guest shell.',
    default_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
    effective_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
  }),
  ms({
    id: 'guest.shell.lang', category: 'Guest Environment', name: 'LANG', setting_type: 'text',
    description: 'Locale for the guest shell.',
    default_value: 'C', effective_value: 'C',
  }),
  ms({
    id: 'guest.tls.ca_bundle', category: 'Guest Environment', name: 'CA bundle path', setting_type: 'text',
    description: 'Path to the CA certificate bundle in the guest.',
    default_value: '/etc/ssl/certs/ca-certificates.crt',
    effective_value: '/etc/ssl/certs/ca-certificates.crt',
  }),
  // -- Network --
  ms({
    id: 'network.default_action', category: 'Network', name: 'Default action', setting_type: 'text',
    description: 'Action for domains not in any allow/block list.',
    default_value: 'deny', effective_value: 'deny',
    corp_locked: true, source: 'corp',
    metadata: { domains: [], choices: ['allow', 'deny'], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'vm.log_bodies', category: 'VM', name: 'Log request bodies', setting_type: 'bool',
    description: 'Capture request/response bodies in telemetry.',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'vm.max_body_capture', category: 'VM', name: 'Max body capture', setting_type: 'number',
    description: 'Maximum bytes of body to capture in telemetry.',
    default_value: 4096, effective_value: 4096,
    metadata: { domains: [], choices: [], min: 0, max: 1048576, rules: {} },
  }),
  ms({
    id: 'network.custom_allow', category: 'Network', name: 'Custom allowed domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to allow. Wildcards supported (*.example.com).',
    default_value: 'elie.net, *.elie.net', effective_value: 'elie.net, *.elie.net',
  }),
  ms({
    id: 'network.custom_block', category: 'Network', name: 'Custom blocked domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to block. Takes priority over custom allow list.',
    default_value: '', effective_value: '',
  }),
  // -- Session (in VM category) --
  ms({
    id: 'vm.retention_days', category: 'VM', name: 'Session retention', setting_type: 'number',
    description: 'Number of days to retain session data.',
    default_value: 30, effective_value: 30,
    metadata: { domains: [], choices: [], min: 1, max: 365, rules: {} },
  }),
  ms({
    id: 'vm.max_sessions', category: 'VM', name: 'Maximum sessions', setting_type: 'number',
    description: 'Keep at most this many sessions (oldest culled first).',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 10000, rules: {} },
  }),
  ms({
    id: 'vm.max_disk_gb', category: 'VM', name: 'Maximum disk usage', setting_type: 'number',
    description: 'Maximum total disk usage for all sessions in GB.',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 1000, rules: {} },
  }),
  // -- Appearance --
  ms({
    id: 'appearance.dark_mode', category: 'Appearance', name: 'Dark mode', setting_type: 'bool',
    description: 'Use dark color scheme in the UI.',
    default_value: true, effective_value: true,
  }),
  ms({
    id: 'appearance.font_size', category: 'Appearance', name: 'Font size', setting_type: 'number',
    description: 'Terminal font size in pixels.',
    default_value: 14, effective_value: 14,
    metadata: { domains: [], choices: [], min: 8, max: 32, rules: {} },
  }),
  // -- VM --
  ms({
    id: 'vm.scratch_disk_size_gb', category: 'VM', name: 'Scratch disk size', setting_type: 'number',
    description: 'Size of the ephemeral scratch disk in GB.',
    default_value: 8, effective_value: 8,
    metadata: { domains: [], choices: [], min: 1, max: 128, rules: {} },
  }),
];

/** Recompute `enabled` flags based on parent toggle values. */
function recomputeEnabled() {
  const values = new Map<string, boolean>();
  for (const s of mockSettings) {
    if (typeof s.effective_value === 'boolean') {
      values.set(s.id, s.effective_value as boolean);
    }
  }
  for (const s of mockSettings) {
    if (s.enabled_by) {
      s.enabled = values.get(s.enabled_by) ?? false;
    }
  }
}

/** Compute lint issues dynamically from current mock settings. */
function computeMockLint(): ConfigIssue[] {
  const issues: ConfigIssue[] = [];
  for (const s of mockSettings) {
    if (s.setting_type === 'apikey' && s.enabled_by) {
      const toggle = mockSettings.find(t => t.id === s.enabled_by);
      if (toggle?.effective_value === true && !String(s.effective_value).trim()) {
        issues.push({
          id: s.id,
          severity: 'warning',
          message: `${s.id}: provider is enabled but API key is empty`,
        });
      }
    }
  }
  return issues;
}

// Set initial enabled flags from the declared settings.
recomputeEnabled();

// Helper: wrap a flat ResolvedSetting into a SettingsLeaf node.
function leaf(s: ResolvedSetting): SettingsNode {
  return { kind: 'leaf', ...s };
}

function buildMockTree(): SettingsNode[] {
  return [
  {
    kind: 'group', key: 'ai', name: 'AI Providers', description: 'AI model provider configuration',
    collapsed: false, children: [
      {
        kind: 'group', key: 'ai.anthropic', name: 'Anthropic', description: 'Claude Code AI agent',
        enabled_by: 'ai.anthropic.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'ai.anthropic.allow')!),
          leaf(mockSettings.find(s => s.id === 'ai.anthropic.api_key')!),
          leaf(mockSettings.find(s => s.id === 'ai.anthropic.domains')!),
          {
            kind: 'group', key: 'ai.anthropic.claude', name: 'Claude Code', description: 'Claude Code configuration files',
            collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'ai.anthropic.claude.settings_json')!),
              leaf(mockSettings.find(s => s.id === 'ai.anthropic.claude.state_json')!),
            ],
          },
        ],
      },
      {
        kind: 'group', key: 'ai.openai', name: 'OpenAI', description: 'OpenAI API provider',
        enabled_by: 'ai.openai.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'ai.openai.allow')!),
          leaf(mockSettings.find(s => s.id === 'ai.openai.api_key')!),
          leaf(mockSettings.find(s => s.id === 'ai.openai.domains')!),
        ],
      },
      {
        kind: 'group', key: 'ai.google', name: 'Google AI', description: 'Google Gemini AI provider',
        enabled_by: 'ai.google.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'ai.google.allow')!),
          leaf(mockSettings.find(s => s.id === 'ai.google.api_key')!),
          leaf(mockSettings.find(s => s.id === 'ai.google.domains')!),
          {
            kind: 'group', key: 'ai.google.gemini', name: 'Gemini CLI', description: 'Gemini CLI configuration files',
            collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'ai.google.gemini.settings_json')!),
              leaf(mockSettings.find(s => s.id === 'ai.google.gemini.projects_json')!),
              leaf(mockSettings.find(s => s.id === 'ai.google.gemini.trusted_folders_json')!),
              leaf(mockSettings.find(s => s.id === 'ai.google.gemini.installation_id')!),
            ],
          },
        ],
      },
    ],
  },
  {
    kind: 'group', key: 'registry', name: 'Package Registries', description: 'Package manager and code hosting access',
    collapsed: false, children: [
      {
        kind: 'group', key: 'registry.github', name: 'GitHub', description: 'GitHub and GitHub-hosted content',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'registry.github.allow')!)],
      },
      {
        kind: 'group', key: 'registry.npm', name: 'npm', description: 'npm package registry',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'registry.npm.allow')!)],
      },
      {
        kind: 'group', key: 'registry.pypi', name: 'PyPI', description: 'Python Package Index',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'registry.pypi.allow')!)],
      },
      {
        kind: 'group', key: 'registry.crates', name: 'crates.io', description: 'Rust crate registry',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'registry.crates.allow')!)],
      },
    ],
  },
  {
    kind: 'group', key: 'search', name: 'Search', description: 'Web search engine access',
    collapsed: false, children: [
      {
        kind: 'group', key: 'search.google', name: 'Google Search', description: 'Google web search',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'search.google.allow')!)],
      },
      {
        kind: 'group', key: 'search.perplexity', name: 'Perplexity', description: 'Perplexity AI search',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'search.perplexity.allow')!)],
      },
      {
        kind: 'group', key: 'search.firecrawl', name: 'Firecrawl', description: 'Firecrawl web scraping API',
        collapsed: false, children: [leaf(mockSettings.find(s => s.id === 'search.firecrawl.allow')!)],
      },
    ],
  },
  {
    kind: 'group', key: 'guest', name: 'Guest Environment', description: 'Guest VM shell and environment configuration',
    collapsed: false, children: [
      {
        kind: 'group', key: 'guest.shell', name: 'Shell', description: 'Guest shell settings',
        collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'guest.shell.term')!),
          leaf(mockSettings.find(s => s.id === 'guest.shell.home')!),
          leaf(mockSettings.find(s => s.id === 'guest.shell.path')!),
          leaf(mockSettings.find(s => s.id === 'guest.shell.lang')!),
        ],
      },
      {
        kind: 'group', key: 'guest.tls', name: 'TLS', description: 'TLS certificate configuration',
        collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'guest.tls.ca_bundle')!),
        ],
      },
    ],
  },
  {
    kind: 'group', key: 'network', name: 'Network', description: 'Network access control and domain filtering',
    collapsed: false, children: [
      leaf(mockSettings.find(s => s.id === 'network.default_action')!),
      leaf(mockSettings.find(s => s.id === 'network.custom_allow')!),
      leaf(mockSettings.find(s => s.id === 'network.custom_block')!),
    ],
  },
  {
    kind: 'group', key: 'vm', name: 'VM', description: 'Virtual machine resource configuration',
    collapsed: false, children: [
      leaf(mockSettings.find(s => s.id === 'vm.log_bodies')!),
      leaf(mockSettings.find(s => s.id === 'vm.max_body_capture')!),
      leaf(mockSettings.find(s => s.id === 'vm.retention_days')!),
      leaf(mockSettings.find(s => s.id === 'vm.max_sessions')!),
      leaf(mockSettings.find(s => s.id === 'vm.max_disk_gb')!),
      leaf(mockSettings.find(s => s.id === 'vm.scratch_disk_size_gb')!),
    ],
  },
  {
    kind: 'group', key: 'appearance', name: 'Appearance', description: 'UI appearance and display settings',
    collapsed: false, children: [
      leaf(mockSettings.find(s => s.id === 'appearance.dark_mode')!),
      leaf(mockSettings.find(s => s.id === 'appearance.font_size')!),
    ],
  },
  ];
}

const MOCK_VM_STATE: VmStateResponse = {
  state: 'Running',
  elapsed_ms: 45000,
  history: [
    { from: 'Created', to: 'Booting', trigger: 'vm_started', duration_ms: 50 },
    { from: 'Booting', to: 'WaitingForAgent', trigger: 'kernel_boot', duration_ms: 1200 },
    { from: 'WaitingForAgent', to: 'Configuring', trigger: 'agent_connected', duration_ms: 800 },
    { from: 'Configuring', to: 'Running', trigger: 'boot_ready_received', duration_ms: 200 },
  ],
};

// ---------------------------------------------------------------------------
// Exported mock API (non-SQL commands only)
// ---------------------------------------------------------------------------

export const mockApi = {
  vmStatus: async () => 'running',
  serialInput: async (_input: string) => {},
  terminalResize: async (_cols: number, _rows: number) => {},
  getGuestConfig: async (): Promise<GuestConfigResponse> => ({ env: { TERM: 'xterm-256color', HOME: '/root' } }),
  getNetworkPolicy: async (): Promise<NetworkPolicyResponse> => ({
    allow: [
      'github.com', '*.github.com', '*.githubusercontent.com',
      'registry.npmjs.org', '*.npmjs.org',
      'pypi.org', 'files.pythonhosted.org',
      'crates.io', 'static.crates.io',
      '*.googleapis.com',
      'www.google.com', 'google.com',
      'elie.net', '*.elie.net',
    ],
    block: [
      '*.anthropic.com', '*.claude.com',
      '*.openai.com',
      'perplexity.ai', '*.perplexity.ai',
      'firecrawl.dev', 'api.firecrawl.dev',
    ],
    default_action: 'deny',
    corp_managed: false,
    conflicts: [],
  }),
  setGuestEnv: async (_key: string, _value: string) => {},
  removeGuestEnv: async (_key: string) => {},
  getSettings: async () => mockSettings.map(s => ({ ...s })),
  getSettingsTree: async () => buildMockTree(),
  lintConfig: async () => computeMockLint(),
  updateSetting: async (id: string, value: any) => {
    const s = mockSettings.find(s => s.id === id);
    if (!s || s.corp_locked) return;
    s.effective_value = value;
    s.source = 'user';
    s.modified = new Date().toISOString();
    recomputeEnabled();
  },
  getVmState: async () => MOCK_VM_STATE,
  getSessionInfo: async (): Promise<SessionInfo> => ({
    session_id: '20260225-143052-a7f3',
    mode: 'gui',
    uptime_ms: 45000,
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 23,
    allowed_requests: 17,
    denied_requests: 6,
    error_requests: 0,
    bytes_sent: 45000,
    bytes_received: 128000,
    model_call_count: 12,
    total_input_tokens: 45200,
    total_output_tokens: 12800,
    total_usage_details: {},
    total_tool_calls: 67,
    total_estimated_cost_usd: 0.42,
  }),

  // Event listeners return no-op unsubscribers in mock mode
  onSerialOutput: async (_cb: (data: number[]) => void) => () => {},
  onVmStateChanged: async (_cb: (state: string) => void) => () => {},
  onTerminalSourceChanged: async (_cb: (source: string) => void) => () => {},
  onDownloadProgress: async (_cb: (progress: any) => void) => () => {},
};

// ---------------------------------------------------------------------------
// sql.js-backed fixture queries for mock mode
// ---------------------------------------------------------------------------

import initSqlJs, { type Database } from 'sql.js';

let fixtureDb: Database | null = null;
let fixtureLoading: Promise<Database> | null = null;

async function getFixtureDb(): Promise<Database> {
  if (fixtureDb) return fixtureDb;
  if (fixtureLoading) return fixtureLoading;
  fixtureLoading = (async () => {
    const SQL = await initSqlJs({
      locateFile: (file: string) => `/node_modules/sql.js/dist/${file}`,
    });
    const resp = await fetch('/fixtures/test.db');
    const buf = await resp.arrayBuffer();
    fixtureDb = new SQL.Database(new Uint8Array(buf));
    return fixtureDb;
  })();
  return fixtureLoading;
}

function runQuery(db: Database, sql: string, params?: unknown[]): QueryResult {
  const stmt = db.prepare(sql);
  if (params && params.length > 0) {
    stmt.bind(params as any);
  }
  const columns: string[] = stmt.getColumnNames();
  const rows: unknown[][] = [];
  while (stmt.step()) {
    rows.push(stmt.get());
  }
  stmt.free();
  return { columns, rows };
}

/** Run a query against the fixture session DB (test.db). */
export async function queryFixture(sql: string, params?: unknown[]): Promise<QueryResult> {
  const db = await getFixtureDb();
  return runQuery(db, sql, params);
}

/** Run a query against fixture -- same DB in mock mode (no separate main.db). */
export async function queryFixtureMain(sql: string, params?: unknown[]): Promise<QueryResult> {
  const db = await getFixtureDb();
  return runQuery(db, sql, params);
}
