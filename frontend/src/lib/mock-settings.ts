// Mock settings data matching the real backend tree format.
// Source: config/defaults.json -- same IDs, types, metadata, and tree hierarchy.
// Do not simplify or fabricate data; this must match what the backend produces.

import type { ResolvedSetting, SettingsNode, SettingsResponse, McpServerInfo, McpToolInfo, McpPolicyInfo } from './types/settings';

// Helper: creates a mock setting with sensible defaults for empty fields.
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

// Helper: wrap a flat ResolvedSetting into a SettingsLeaf node.
function leaf(s: ResolvedSetting): SettingsNode {
  return { kind: 'leaf', ...s };
}

export let mockSettings: ResolvedSetting[] = [
  ms({ id: 'app.auto_update', category: 'App', name: 'Auto-check for updates', setting_type: 'bool', description: 'Check for new Capsem versions on launch', default_value: true, effective_value: true }),
  ms({ id: 'ai.anthropic.allow', category: 'Anthropic', name: 'Allow Anthropic', setting_type: 'bool', description: 'Enable API access to Anthropic (*.anthropic.com).', default_value: true, effective_value: true, metadata: { domains: [], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: true, put: false, delete: false, other: false } } } }),
  ms({ id: 'ai.anthropic.api_key', category: 'Anthropic', name: 'Anthropic API Key', setting_type: 'apikey', description: 'API key for Anthropic. Injected as ANTHROPIC_API_KEY env var.', default_value: '', effective_value: '', enabled_by: 'ai.anthropic.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://console.anthropic.com/settings/keys', prefix: 'sk-ant-' } }),
  ms({ id: 'ai.anthropic.domains', category: 'Anthropic', name: 'Anthropic Domains', setting_type: 'text', description: 'Comma-separated domain patterns. Wildcards (*.example.com) match all subdomains.', default_value: '*.anthropic.com, *.claude.com', effective_value: '*.anthropic.com, *.claude.com', enabled_by: 'ai.anthropic.allow', enabled: false }),
  ms({ id: 'ai.anthropic.claude.settings_json', category: 'Claude Code', name: 'Claude Code settings.json', setting_type: 'file', description: 'Content for /root/.claude/settings.json. Bypass permissions, disable telemetry/updates for sandboxed execution.', default_value: { path: '/root/.claude/settings.json', content: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}' }, effective_value: { path: '/root/.claude/settings.json', content: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}' }, enabled_by: 'ai.anthropic.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' } }),
  ms({ id: 'ai.anthropic.claude.state_json', category: 'Claude Code', name: 'Claude Code state (.claude.json)', setting_type: 'file', description: 'Content for /root/.claude.json. Skips onboarding, trust dialogs, and keybinding prompts.', default_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true,"hasTrustDialogHooksAccepted":true,"shiftEnterKeyBindingInstalled":true,"theme":"dark","numStartups":1}' }, effective_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true,"hasTrustDialogHooksAccepted":true,"shiftEnterKeyBindingInstalled":true,"theme":"dark","numStartups":1}' }, enabled_by: 'ai.anthropic.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' } }),
  ms({ id: 'ai.anthropic.claude.credentials_json', category: 'Claude Code', name: 'Claude Code OAuth credentials', setting_type: 'file', description: 'Content for /root/.claude/.credentials.json. OAuth tokens for subscription-based auth (Pro/Max).', default_value: { path: '/root/.claude/.credentials.json', content: '' }, effective_value: { path: '/root/.claude/.credentials.json', content: '' }, enabled_by: 'ai.anthropic.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' } }),
  ms({ id: 'ai.google.allow', category: 'Google AI', name: 'Allow Google AI', setting_type: 'bool', description: 'Enable API access to Google AI (*.googleapis.com).', default_value: true, effective_value: true, metadata: { domains: [], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: true, put: false, delete: false, other: false } } } }),
  ms({ id: 'ai.google.api_key', category: 'Google AI', name: 'Google AI API Key', setting_type: 'apikey', description: 'API key for Google AI. Injected as GEMINI_API_KEY env var.', default_value: '', effective_value: '', enabled_by: 'ai.google.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://aistudio.google.com/apikey', prefix: 'AIza' } }),
  ms({ id: 'ai.google.domains', category: 'Google AI', name: 'Google AI Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: '*.googleapis.com', effective_value: '*.googleapis.com', enabled_by: 'ai.google.allow', enabled: false }),
  ms({ id: 'ai.google.gemini.settings_json', category: 'Gemini CLI', name: 'Gemini CLI settings.json', setting_type: 'file', description: 'Content for /root/.gemini/settings.json.', default_value: { path: '/root/.gemini/settings.json', content: '{"homeDirectoryWarningDismissed":true,"general":{"disableAutoUpdate":true},"telemetry":{"enabled":false}}' }, effective_value: { path: '/root/.gemini/settings.json', content: '{"homeDirectoryWarningDismissed":true,"general":{"disableAutoUpdate":true},"telemetry":{"enabled":false}}' }, enabled_by: 'ai.google.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' } }),
  ms({ id: 'ai.openai.allow', category: 'OpenAI', name: 'Allow OpenAI', setting_type: 'bool', description: 'Enable API access to OpenAI (*.openai.com).', default_value: true, effective_value: true, metadata: { domains: [], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: true, put: false, delete: false, other: false } } } }),
  ms({ id: 'ai.openai.api_key', category: 'OpenAI', name: 'OpenAI API Key', setting_type: 'apikey', description: 'API key for OpenAI. Injected as OPENAI_API_KEY env var.', default_value: '', effective_value: '', enabled_by: 'ai.openai.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://platform.openai.com/api-keys', prefix: 'sk-' } }),
  ms({ id: 'ai.openai.domains', category: 'OpenAI', name: 'OpenAI Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: '*.openai.com', effective_value: '*.openai.com', enabled_by: 'ai.openai.allow', enabled: false }),
  ms({ id: 'ai.openai.codex.config_toml', category: 'Codex CLI', name: 'Codex CLI config.toml', setting_type: 'file', description: 'Content for /root/.codex/config.toml.', default_value: { path: '/root/.codex/config.toml', content: '[mcp_servers.capsem]\ncommand = "/run/capsem-mcp-server"' }, effective_value: { path: '/root/.codex/config.toml', content: '[mcp_servers.capsem]\ncommand = "/run/capsem-mcp-server"' }, enabled_by: 'ai.openai.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'toml' } }),
  ms({ id: 'repository.git.identity.author_name', category: 'Git Identity', name: 'Author name', setting_type: 'text', description: 'Name used for git commits.', default_value: '', effective_value: '' }),
  ms({ id: 'repository.git.identity.author_email', category: 'Git Identity', name: 'Author email', setting_type: 'text', description: 'Email used for git commits.', default_value: '', effective_value: '' }),
  ms({ id: 'repository.providers.github.allow', category: 'GitHub', name: 'Allow GitHub', setting_type: 'bool', description: 'Enable access to GitHub and GitHub-hosted content.', default_value: true, effective_value: true, metadata: { domains: ['github.com', '*.github.com', '*.githubusercontent.com'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: true, put: false, delete: false, other: false } } } }),
  ms({ id: 'repository.providers.github.domains', category: 'GitHub', name: 'GitHub Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'github.com, *.github.com, *.githubusercontent.com', effective_value: 'github.com, *.github.com, *.githubusercontent.com', enabled_by: 'repository.providers.github.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'repository.providers.github.token', category: 'GitHub', name: 'GitHub Token', setting_type: 'apikey', description: 'Personal access token for git push over HTTPS.', default_value: '', effective_value: '', enabled_by: 'repository.providers.github.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://github.com/settings/tokens', prefix: 'ghp_' } }),
  ms({ id: 'repository.providers.gitlab.allow', category: 'GitLab', name: 'Allow GitLab', setting_type: 'bool', description: 'Enable access to GitLab and GitLab-hosted content.', default_value: false, effective_value: false, metadata: { domains: ['gitlab.com', '*.gitlab.com'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: true, put: false, delete: false, other: false } } } }),
  ms({ id: 'repository.providers.gitlab.domains', category: 'GitLab', name: 'GitLab Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'gitlab.com, *.gitlab.com', effective_value: 'gitlab.com, *.gitlab.com', enabled_by: 'repository.providers.gitlab.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'repository.providers.gitlab.token', category: 'GitLab', name: 'GitLab Token', setting_type: 'apikey', description: 'Personal access token for git push over HTTPS.', default_value: '', effective_value: '', enabled_by: 'repository.providers.gitlab.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://gitlab.com/-/user_settings/personal_access_tokens', prefix: 'glpat-' } }),
  ms({ id: 'security.web.allow_read', category: 'Web', name: 'Allow read requests', setting_type: 'bool', description: 'Allow GET/HEAD/OPTIONS for domains not in any allow/block list.', default_value: false, effective_value: false }),
  ms({ id: 'security.web.allow_write', category: 'Web', name: 'Allow write requests', setting_type: 'bool', description: 'Allow POST/PUT/DELETE/PATCH for domains not in any allow/block list.', default_value: false, effective_value: false }),
  ms({ id: 'security.web.custom_allow', category: 'Web', name: 'Allowed domains', setting_type: 'text', description: 'Comma-separated domain patterns to allow.', default_value: 'elie.net, *.elie.net, ash-speed.hetzner.com, en.wikipedia.org, *.wikipedia.org', effective_value: 'elie.net, *.elie.net, ash-speed.hetzner.com, en.wikipedia.org, *.wikipedia.org', metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.web.custom_block', category: 'Web', name: 'Blocked domains', setting_type: 'text', description: 'Comma-separated domain patterns to block. Takes priority over custom allow list.', default_value: '', effective_value: '', metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.search.google.allow', category: 'Google', name: 'Allow Google', setting_type: 'bool', description: 'Enable access to Google web search.', default_value: true, effective_value: true, metadata: { domains: ['www.google.com', 'google.com'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.search.google.domains', category: 'Google', name: 'Google Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'www.google.com, google.com', effective_value: 'www.google.com, google.com', enabled_by: 'security.services.search.google.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.search.bing.allow', category: 'Bing', name: 'Allow Bing', setting_type: 'bool', description: 'Enable access to Bing web search.', default_value: false, effective_value: false, metadata: { domains: ['www.bing.com', 'bing.com'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.search.bing.domains', category: 'Bing', name: 'Bing Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'www.bing.com, bing.com', effective_value: 'www.bing.com, bing.com', enabled_by: 'security.services.search.bing.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.search.duckduckgo.allow', category: 'DuckDuckGo', name: 'Allow DuckDuckGo', setting_type: 'bool', description: 'Enable access to DuckDuckGo web search.', default_value: false, effective_value: false, metadata: { domains: ['duckduckgo.com', '*.duckduckgo.com'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.search.duckduckgo.domains', category: 'DuckDuckGo', name: 'DuckDuckGo Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'duckduckgo.com, *.duckduckgo.com', effective_value: 'duckduckgo.com, *.duckduckgo.com', enabled_by: 'security.services.search.duckduckgo.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.registry.npm.allow', category: 'npm', name: 'Allow npm', setting_type: 'bool', description: 'Enable access to npm.', default_value: true, effective_value: true, metadata: { domains: ['registry.npmjs.org', '*.npmjs.org'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.registry.npm.domains', category: 'npm', name: 'npm Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'registry.npmjs.org, *.npmjs.org', effective_value: 'registry.npmjs.org, *.npmjs.org', enabled_by: 'security.services.registry.npm.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.registry.pypi.allow', category: 'PyPI', name: 'Allow PyPI', setting_type: 'bool', description: 'Enable access to PyPI.', default_value: true, effective_value: true, metadata: { domains: ['pypi.org', 'files.pythonhosted.org'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.registry.pypi.domains', category: 'PyPI', name: 'PyPI Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'pypi.org, files.pythonhosted.org', effective_value: 'pypi.org, files.pythonhosted.org', enabled_by: 'security.services.registry.pypi.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'security.services.registry.crates.allow', category: 'crates.io', name: 'Allow crates.io', setting_type: 'bool', description: 'Enable access to crates.io.', default_value: true, effective_value: true, metadata: { domains: ['crates.io', 'static.crates.io'], choices: [], min: null, max: null, rules: { default: { domains: [], path: null, get: true, post: false, put: false, delete: false, other: false } } } }),
  ms({ id: 'security.services.registry.crates.domains', category: 'crates.io', name: 'crates.io Domains', setting_type: 'text', description: 'Comma-separated domain patterns.', default_value: 'crates.io, static.crates.io', effective_value: 'crates.io, static.crates.io', enabled_by: 'security.services.registry.crates.allow', enabled: false, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' } }),
  ms({ id: 'vm.snapshots.auto_max', category: 'Snapshots', name: 'Auto snapshot limit', setting_type: 'number', description: 'Maximum number of automatic rolling snapshots.', default_value: 10, effective_value: 10, metadata: { domains: [], choices: [], min: 1, max: 50, rules: {} } }),
  ms({ id: 'vm.snapshots.manual_max', category: 'Snapshots', name: 'Manual snapshot limit', setting_type: 'number', description: 'Maximum number of named manual snapshots.', default_value: 12, effective_value: 12, metadata: { domains: [], choices: [], min: 1, max: 50, rules: {} } }),
  ms({ id: 'vm.snapshots.auto_interval', category: 'Snapshots', name: 'Auto snapshot interval', setting_type: 'number', description: 'Seconds between automatic snapshots.', default_value: 300, effective_value: 300, metadata: { domains: [], choices: [], min: 30, max: 3600, rules: {} } }),
  ms({ id: 'vm.environment.shell.term', category: 'Shell', name: 'TERM', setting_type: 'text', description: 'Terminal type for the guest shell.', default_value: 'xterm-256color', effective_value: 'xterm-256color' }),
  ms({ id: 'vm.environment.shell.home', category: 'Shell', name: 'HOME', setting_type: 'text', description: 'Home directory for the guest shell.', default_value: '/root', effective_value: '/root' }),
  ms({ id: 'vm.environment.shell.path', category: 'Shell', name: 'PATH', setting_type: 'text', description: 'Executable search path for the guest shell.', default_value: '/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin', effective_value: '/opt/ai-clis/bin:/root/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin' }),
  ms({ id: 'vm.environment.shell.lang', category: 'Shell', name: 'LANG', setting_type: 'text', description: 'Locale for the guest shell.', default_value: 'C', effective_value: 'C' }),
  ms({ id: 'vm.environment.shell.bashrc', category: 'Shell', name: 'Bash configuration', setting_type: 'file', description: 'User shell config sourced at login. Customize prompt, aliases, and functions.', default_value: { path: '/root/.bashrc', content: '# Prompt: green bold "capsem" with blue directory\nPS1=\'\\[\\033[1;32m\\]capsem\\[\\033[0m\\]:\\[\\033[1;34m\\]\\w\\[\\033[0m\\]\\$ \'\n\n# Aliases\nalias ls=\'ls --color=auto\'\nalias ll=\'ls -la --color=auto\'\nalias grep=\'grep --color=auto\'\n' }, effective_value: { path: '/root/.bashrc', content: '# Prompt: green bold "capsem" with blue directory\nPS1=\'\\[\\033[1;32m\\]capsem\\[\\033[0m\\]:\\[\\033[1;34m\\]\\w\\[\\033[0m\\]\\$ \'\n\n# Aliases\nalias ls=\'ls --color=auto\'\nalias ll=\'ls -la --color=auto\'\nalias grep=\'grep --color=auto\'\n' }, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'bash' } }),
  ms({ id: 'vm.environment.shell.tmux_conf', category: 'Shell', name: 'tmux configuration', setting_type: 'file', description: 'tmux terminal multiplexer config.', default_value: { path: '/root/.tmux.conf', content: 'set -g default-terminal "tmux-256color"\nset -g mouse on\nset -g escape-time 0\nset -g history-limit 50000\n' }, effective_value: { path: '/root/.tmux.conf', content: 'set -g default-terminal "tmux-256color"\nset -g mouse on\nset -g escape-time 0\nset -g history-limit 50000\n' }, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'conf' } }),
  ms({ id: 'vm.environment.ssh.public_key', category: 'SSH', name: 'SSH public key', setting_type: 'text', description: 'Public key injected as /root/.ssh/authorized_keys in the guest VM.', default_value: '', effective_value: '' }),
  ms({ id: 'vm.environment.tls.ca_bundle', category: 'TLS', name: 'CA bundle path', setting_type: 'text', description: 'Path to the CA certificate bundle in the guest.', default_value: '/etc/ssl/certs/ca-certificates.crt', effective_value: '/etc/ssl/certs/ca-certificates.crt' }),
  ms({ id: 'vm.resources.cpu_count', category: 'Resources', name: 'CPU cores', setting_type: 'number', description: 'Number of CPU cores allocated to the VM.', default_value: 4, effective_value: 4, metadata: { domains: [], choices: [], min: 1, max: 8, rules: {} } }),
  ms({ id: 'vm.resources.ram_gb', category: 'Resources', name: 'RAM', setting_type: 'number', description: 'Amount of RAM allocated to the VM in GB.', default_value: 4, effective_value: 4, metadata: { domains: [], choices: [], min: 1, max: 16, rules: {} } }),
  ms({ id: 'vm.resources.scratch_disk_size_gb', category: 'Resources', name: 'Scratch disk size', setting_type: 'number', description: 'Size of the ephemeral scratch disk in GB.', default_value: 16, effective_value: 16, metadata: { domains: [], choices: [], min: 1, max: 128, rules: {} } }),
  ms({ id: 'vm.resources.log_bodies', category: 'Resources', name: 'Log request bodies', setting_type: 'bool', description: 'Capture request/response bodies in telemetry.', default_value: false, effective_value: false }),
  ms({ id: 'vm.resources.max_body_capture', category: 'Resources', name: 'Max body capture', setting_type: 'number', description: 'Maximum bytes of body to capture in telemetry.', default_value: 4096, effective_value: 4096, metadata: { domains: [], choices: [], min: 0, max: 1048576, rules: {} } }),
  ms({ id: 'vm.resources.retention_days', category: 'Resources', name: 'Session retention', setting_type: 'number', description: 'Number of days to retain session data.', default_value: 30, effective_value: 30, metadata: { domains: [], choices: [], min: 1, max: 365, rules: {} } }),
  ms({ id: 'vm.resources.max_sessions', category: 'Resources', name: 'Maximum sessions', setting_type: 'number', description: 'Keep at most this many sessions (oldest culled first).', default_value: 100, effective_value: 100, metadata: { domains: [], choices: [], min: 1, max: 10000, rules: {} } }),
  ms({ id: 'appearance.dark_mode', category: 'Appearance', name: 'Dark mode', setting_type: 'bool', description: 'Use dark color scheme in the UI.', default_value: true, effective_value: true, metadata: { domains: [], choices: [], min: null, max: null, rules: {}, side_effect: 'toggle_theme' } }),
  ms({ id: 'appearance.font_size', category: 'Appearance', name: 'Font size', setting_type: 'number', description: 'Terminal font size in pixels.', default_value: 14, effective_value: 14, metadata: { domains: [], choices: [], min: 8, max: 32, rules: {} } }),
];

/** Recompute `enabled` flags based on parent toggle values. */
export function recomputeEnabled() {
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

function find(id: string): ResolvedSetting {
  const s = mockSettings.find(s => s.id === id);
  if (!s) throw new Error(`Mock setting not found: ${id}`);
  return s;
}

export function buildMockTree(): SettingsNode[] {
  recomputeEnabled();
  return [
    { kind: 'group', enabled: true, key: 'app', name: 'App', description: 'Application settings', collapsed: false, children: [
      leaf(find('app.auto_update')),
      { kind: 'action', key: 'app.check_update', name: 'Check for updates', description: 'Manually check if a new version is available', action: 'check_update' },
    ]},
    { kind: 'group', enabled: true, key: 'ai', name: 'AI Providers', description: 'AI model provider configuration', collapsed: false, children: [
      { kind: 'group', enabled: true, key: 'ai.anthropic', name: 'Anthropic', description: 'Claude Code AI agent', enabled_by: 'ai.anthropic.allow', collapsed: false, children: [
        leaf(find('ai.anthropic.allow')),
        leaf(find('ai.anthropic.api_key')),
        leaf(find('ai.anthropic.domains')),
        { kind: 'group', enabled: true, key: 'ai.anthropic.claude', name: 'Claude Code', description: 'Claude Code configuration files', collapsed: false, children: [
          leaf(find('ai.anthropic.claude.settings_json')),
          leaf(find('ai.anthropic.claude.state_json')),
          leaf(find('ai.anthropic.claude.credentials_json')),
        ]},
      ]},
      { kind: 'group', enabled: true, key: 'ai.google', name: 'Google AI', description: 'Google Gemini AI provider', enabled_by: 'ai.google.allow', collapsed: false, children: [
        leaf(find('ai.google.allow')),
        leaf(find('ai.google.api_key')),
        leaf(find('ai.google.domains')),
        { kind: 'group', enabled: true, key: 'ai.google.gemini', name: 'Gemini CLI', description: 'Gemini CLI configuration files', collapsed: false, children: [
          leaf(find('ai.google.gemini.settings_json')),
        ]},
      ]},
      { kind: 'group', enabled: true, key: 'ai.openai', name: 'OpenAI', description: 'OpenAI API provider', enabled_by: 'ai.openai.allow', collapsed: false, children: [
        leaf(find('ai.openai.allow')),
        leaf(find('ai.openai.api_key')),
        leaf(find('ai.openai.domains')),
        { kind: 'group', enabled: true, key: 'ai.openai.codex', name: 'Codex CLI', description: 'Codex CLI configuration files', collapsed: false, children: [
          leaf(find('ai.openai.codex.config_toml')),
        ]},
      ]},
    ]},
    { kind: 'group', enabled: true, key: 'repository', name: 'Repositories', description: 'Code hosting and git configuration', collapsed: false, children: [
      { kind: 'group', enabled: true, key: 'repository.git.identity', name: 'Git Identity', description: 'Author name and email for commits inside the VM', collapsed: false, children: [
        leaf(find('repository.git.identity.author_name')),
        leaf(find('repository.git.identity.author_email')),
      ]},
      { kind: 'group', enabled: true, key: 'repository.providers', name: 'Providers', description: 'Code hosting platforms', collapsed: false, children: [
        { kind: 'group', enabled: true, key: 'repository.providers.github', name: 'GitHub', description: 'GitHub and GitHub-hosted content', enabled_by: 'repository.providers.github.allow', collapsed: false, children: [
          leaf(find('repository.providers.github.allow')),
          leaf(find('repository.providers.github.domains')),
          leaf(find('repository.providers.github.token')),
        ]},
        { kind: 'group', enabled: true, key: 'repository.providers.gitlab', name: 'GitLab', description: 'GitLab and GitLab-hosted content', enabled_by: 'repository.providers.gitlab.allow', collapsed: false, children: [
          leaf(find('repository.providers.gitlab.allow')),
          leaf(find('repository.providers.gitlab.domains')),
          leaf(find('repository.providers.gitlab.token')),
        ]},
      ]},
    ]},
    { kind: 'group', enabled: true, key: 'security', name: 'Security', description: 'Network access control, web services, and security presets', collapsed: false, children: [
      { kind: 'action', key: 'security.preset', name: 'Security Preset', description: 'Predefined security configurations', action: 'preset_select' },
      { kind: 'group', enabled: true, key: 'security.web', name: 'Web', description: 'Default actions for unknown domains', collapsed: false, children: [
        leaf(find('security.web.allow_read')),
        leaf(find('security.web.allow_write')),
        leaf(find('security.web.custom_allow')),
        leaf(find('security.web.custom_block')),
      ]},
      { kind: 'group', enabled: true, key: 'security.services', name: 'Services', description: 'Search engines and package registries', collapsed: false, children: [
        { kind: 'group', enabled: true, key: 'security.services.search', name: 'Search Engines', description: 'Web search engine access', collapsed: false, children: [
          { kind: 'group', enabled: true, key: 'security.services.search.google', name: 'Google', description: 'Google web search', enabled_by: 'security.services.search.google.allow', collapsed: false, children: [
            leaf(find('security.services.search.google.allow')),
            leaf(find('security.services.search.google.domains')),
          ]},
          { kind: 'group', enabled: true, key: 'security.services.search.bing', name: 'Bing', description: 'Bing web search', enabled_by: 'security.services.search.bing.allow', collapsed: false, children: [
            leaf(find('security.services.search.bing.allow')),
            leaf(find('security.services.search.bing.domains')),
          ]},
          { kind: 'group', enabled: true, key: 'security.services.search.duckduckgo', name: 'DuckDuckGo', description: 'DuckDuckGo web search', enabled_by: 'security.services.search.duckduckgo.allow', collapsed: false, children: [
            leaf(find('security.services.search.duckduckgo.allow')),
            leaf(find('security.services.search.duckduckgo.domains')),
          ]},
        ]},
        { kind: 'group', enabled: true, key: 'security.services.registry', name: 'Package Registries', description: 'Package manager registries', collapsed: false, children: [
          { kind: 'group', enabled: true, key: 'security.services.registry.npm', name: 'npm', description: 'npm package registry', enabled_by: 'security.services.registry.npm.allow', collapsed: false, children: [
            leaf(find('security.services.registry.npm.allow')),
            leaf(find('security.services.registry.npm.domains')),
          ]},
          { kind: 'group', enabled: true, key: 'security.services.registry.pypi', name: 'PyPI', description: 'PyPI package registry', enabled_by: 'security.services.registry.pypi.allow', collapsed: false, children: [
            leaf(find('security.services.registry.pypi.allow')),
            leaf(find('security.services.registry.pypi.domains')),
          ]},
          { kind: 'group', enabled: true, key: 'security.services.registry.crates', name: 'crates.io', description: 'crates.io package registry', enabled_by: 'security.services.registry.crates.allow', collapsed: false, children: [
            leaf(find('security.services.registry.crates.allow')),
            leaf(find('security.services.registry.crates.domains')),
          ]},
        ]},
      ]},
    ]},
    { kind: 'group', enabled: true, key: 'vm', name: 'VM', description: 'Virtual machine configuration', collapsed: false, children: [
      { kind: 'group', enabled: true, key: 'vm.snapshots', name: 'Snapshots', description: 'Automatic and manual workspace snapshot settings', collapsed: false, children: [
        leaf(find('vm.snapshots.auto_max')),
        leaf(find('vm.snapshots.manual_max')),
        leaf(find('vm.snapshots.auto_interval')),
      ]},
      { kind: 'group', enabled: true, key: 'vm.environment', name: 'Environment', description: 'Shell and environment variables', collapsed: false, children: [
        { kind: 'group', enabled: true, key: 'vm.environment.shell', name: 'Shell', description: 'Guest shell settings', collapsed: false, children: [
          leaf(find('vm.environment.shell.term')),
          leaf(find('vm.environment.shell.home')),
          leaf(find('vm.environment.shell.path')),
          leaf(find('vm.environment.shell.lang')),
          leaf(find('vm.environment.shell.bashrc')),
          leaf(find('vm.environment.shell.tmux_conf')),
        ]},
        { kind: 'group', enabled: true, key: 'vm.environment.ssh', name: 'SSH', description: 'SSH key configuration', collapsed: false, children: [
          leaf(find('vm.environment.ssh.public_key')),
        ]},
        { kind: 'group', enabled: true, key: 'vm.environment.tls', name: 'TLS', description: 'TLS certificate configuration', collapsed: false, children: [
          leaf(find('vm.environment.tls.ca_bundle')),
        ]},
      ]},
      { kind: 'group', enabled: true, key: 'vm.resources', name: 'Resources', description: 'Hardware, telemetry, and session limits', collapsed: false, children: [
        leaf(find('vm.resources.cpu_count')),
        leaf(find('vm.resources.ram_gb')),
        leaf(find('vm.resources.scratch_disk_size_gb')),
        leaf(find('vm.resources.log_bodies')),
        leaf(find('vm.resources.max_body_capture')),
        leaf(find('vm.resources.retention_days')),
        leaf(find('vm.resources.max_sessions')),
      ]},
    ]},
    { kind: 'group', enabled: true, key: 'appearance', name: 'Appearance', description: 'UI appearance and display settings', collapsed: false, children: [
      leaf(find('appearance.dark_mode')),
      leaf(find('appearance.font_size')),
    ]},
  ];
}

// ---------------------------------------------------------------------------
// MCP mock data
// ---------------------------------------------------------------------------

export const MOCK_MCP_SERVERS: McpServerInfo[] = [];

export const MOCK_MCP_TOOLS: McpToolInfo[] = [
  {
    namespaced_name: 'fetch_http',
    original_name: 'fetch_http',
    description: 'Fetch a URL and return its content.',
    server_name: 'builtin',
    annotations: { title: 'Fetch HTTP', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: null, approved: true, pin_changed: false,
  },
  {
    namespaced_name: 'grep_http',
    original_name: 'grep_http',
    description: 'Fetch a URL and search its content for a regex pattern.',
    server_name: 'builtin',
    annotations: { title: 'Grep HTTP', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: null, approved: true, pin_changed: false,
  },
  {
    namespaced_name: 'http_headers',
    original_name: 'http_headers',
    description: 'Return HTTP status code and response headers for a URL.',
    server_name: 'builtin',
    annotations: { title: 'HTTP Headers', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: null, approved: true, pin_changed: false,
  },
  {
    namespaced_name: 'snapshots_list',
    original_name: 'snapshots_list',
    description: 'List all workspace snapshots (automatic and manual).',
    server_name: 'builtin',
    annotations: { title: 'List snapshots', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false },
    pin_hash: null, approved: true, pin_changed: false,
  },
  {
    namespaced_name: 'snapshots_create',
    original_name: 'snapshots_create',
    description: 'Create a named workspace snapshot (checkpoint).',
    server_name: 'builtin',
    annotations: { title: 'Create snapshot', read_only_hint: false, destructive_hint: false, idempotent_hint: false, open_world_hint: false },
    pin_hash: null, approved: true, pin_changed: false,
  },
  {
    namespaced_name: 'snapshots_revert',
    original_name: 'snapshots_revert',
    description: 'Revert a file to its state at a specific checkpoint.',
    server_name: 'builtin',
    annotations: { title: 'Revert file', read_only_hint: false, destructive_hint: true, idempotent_hint: true, open_world_hint: false },
    pin_hash: null, approved: true, pin_changed: false,
  },
];

export const MOCK_MCP_POLICY: McpPolicyInfo = {
  global_policy: 'allow',
  default_tool_permission: 'allow',
  blocked_servers: [],
  tool_permissions: {},
};

// ---------------------------------------------------------------------------
// Mock presets
// ---------------------------------------------------------------------------

export const MOCK_PRESETS = [
  {
    id: 'medium',
    name: 'Medium',
    description: 'Allow read-only web, all search engines, MCP tools without confirmation.',
    settings: {
      'security.web.allow_read': true,
      'security.web.allow_write': false,
      'security.services.search.google.allow': true,
      'security.services.search.bing.allow': true,
      'security.services.search.duckduckgo.allow': true,
    },
    mcp: { default_tool_permission: 'allow' },
  },
  {
    id: 'high',
    name: 'High',
    description: 'Block all web access, selective search only, stricter MCP policies.',
    settings: {
      'security.web.allow_read': false,
      'security.web.allow_write': false,
      'security.services.search.google.allow': true,
      'security.services.search.bing.allow': false,
      'security.services.search.duckduckgo.allow': false,
    },
    mcp: { default_tool_permission: 'warn' },
  },
];

// ---------------------------------------------------------------------------
// Build the full mock response
// ---------------------------------------------------------------------------

export function buildMockSettingsResponse(): SettingsResponse {
  return {
    tree: buildMockTree(),
    issues: [
      { id: 'ai.anthropic.api_key', severity: 'warning', message: 'No Anthropic API key configured. Claude Code will not be able to authenticate.', docs_url: 'https://console.anthropic.com/settings/keys' },
      { id: 'ai.google.api_key', severity: 'warning', message: 'No Google AI API key configured. Gemini CLI will not be able to authenticate.', docs_url: 'https://aistudio.google.com/apikey' },
      { id: 'ai.openai.api_key', severity: 'warning', message: 'No OpenAI API key configured. Codex CLI will not be able to authenticate.', docs_url: 'https://platform.openai.com/api-keys' },
    ],
    presets: MOCK_PRESETS,
  };
}
