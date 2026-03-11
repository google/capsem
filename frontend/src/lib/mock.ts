// Mock data for browser-only dev mode (no Tauri backend).
// Active when window.__TAURI_INTERNALS__ is absent.
import type {
  ConfigIssue,
  McpPolicyInfo,
  McpServerInfo,
  McpToolInfo,
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
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://console.anthropic.com/settings/keys', prefix: 'sk-ant-' },
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
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' },
  }),
  ms({
    id: 'ai.anthropic.claude.state_json', category: 'AI Providers', name: 'Claude Code state (.claude.json)', setting_type: 'file',
    description: 'Content for ~/.claude.json. Skips onboarding.',
    default_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}' },
    effective_value: { path: '/root/.claude.json', content: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}' },
    enabled_by: 'ai.anthropic.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' },
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
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://platform.openai.com/api-keys', prefix: 'sk-' },
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
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://aistudio.google.com/apikey', prefix: 'AIza' },
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
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' },
  }),
  ms({
    id: 'ai.google.gemini.projects_json', category: 'AI Providers', name: 'Gemini projects.json', setting_type: 'file',
    description: 'Content for ~/.gemini/projects.json.',
    default_value: { path: '/root/.gemini/projects.json', content: '{"projects":{"/root":"root"}}' },
    effective_value: { path: '/root/.gemini/projects.json', content: '{"projects":{"/root":"root"}}' },
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' },
  }),
  ms({
    id: 'ai.google.gemini.trusted_folders_json', category: 'AI Providers', name: 'Gemini trustedFolders.json', setting_type: 'file',
    description: 'Content for ~/.gemini/trustedFolders.json.',
    default_value: { path: '/root/.gemini/trustedFolders.json', content: '{"/root":"TRUST_FOLDER"}' },
    effective_value: { path: '/root/.gemini/trustedFolders.json', content: '{"/root":"TRUST_FOLDER"}' },
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'json' },
  }),
  ms({
    id: 'ai.google.gemini.installation_id', category: 'AI Providers', name: 'Gemini installation_id', setting_type: 'file',
    description: 'Stable UUID avoids first-run prompts.',
    default_value: { path: '/root/.gemini/installation_id', content: 'capsem-sandbox-00000000-0000-0000-0000-000000000000' },
    effective_value: { path: '/root/.gemini/installation_id', content: 'capsem-sandbox-00000000-0000-0000-0000-000000000000' },
    enabled_by: 'ai.google.allow',
  }),
  // -- Repositories --
  ms({
    id: 'repository.git.identity.author_name', category: 'Repositories', name: 'Author name', setting_type: 'text',
    description: 'Name used for git commits. Injected as GIT_AUTHOR_NAME and GIT_COMMITTER_NAME.',
    default_value: '', effective_value: '',
  }),
  ms({
    id: 'repository.git.identity.author_email', category: 'Repositories', name: 'Author email', setting_type: 'text',
    description: 'Email used for git commits. Injected as GIT_AUTHOR_EMAIL and GIT_COMMITTER_EMAIL.',
    default_value: '', effective_value: '',
  }),
  ms({
    id: 'repository.providers.github.allow', category: 'Repositories', name: 'Allow GitHub', setting_type: 'bool',
    description: 'Enable access to GitHub and GitHub-hosted content.',
    default_value: true, effective_value: true,
    corp_locked: true, source: 'corp',
    metadata: { domains: ['github.com', '*.github.com', '*.githubusercontent.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'repository.providers.github.domains', category: 'Repositories', name: 'GitHub Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'github.com, *.github.com, *.githubusercontent.com',
    effective_value: 'github.com, *.github.com, *.githubusercontent.com',
    enabled_by: 'repository.providers.github.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'repository.providers.github.token', category: 'Repositories', name: 'GitHub Token', setting_type: 'apikey',
    description: 'Personal access token for git push over HTTPS. Injected into .git-credentials.',
    default_value: '', effective_value: '',
    enabled_by: 'repository.providers.github.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://github.com/settings/tokens', prefix: 'ghp_' },
  }),
  ms({
    id: 'repository.providers.gitlab.allow', category: 'Repositories', name: 'Allow GitLab', setting_type: 'bool',
    description: 'Enable access to GitLab and GitLab-hosted content.',
    default_value: false, effective_value: false,
    metadata: { domains: ['gitlab.com', '*.gitlab.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'repository.providers.gitlab.domains', category: 'Repositories', name: 'GitLab Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'gitlab.com, *.gitlab.com',
    effective_value: 'gitlab.com, *.gitlab.com',
    enabled_by: 'repository.providers.gitlab.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'repository.providers.gitlab.token', category: 'Repositories', name: 'GitLab Token', setting_type: 'apikey',
    description: 'Personal access token for git push over HTTPS. Injected into .git-credentials.',
    default_value: '', effective_value: '',
    enabled_by: 'repository.providers.gitlab.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, docs_url: 'https://gitlab.com/-/user_settings/personal_access_tokens', prefix: 'glpat-' },
  }),
  // -- Package Registries --
  ms({
    id: 'registry.debian.allow', category: 'Package Registries', name: 'Allow Debian', setting_type: 'bool',
    description: 'Enable access to Debian package repositories.',
    default_value: true, effective_value: true,
    metadata: { domains: ['deb.debian.org', 'security.debian.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.debian.domains', category: 'Package Registries', name: 'Debian Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'deb.debian.org, security.debian.org',
    effective_value: 'deb.debian.org, security.debian.org',
    enabled_by: 'registry.debian.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'registry.npm.allow', category: 'Package Registries', name: 'Allow npm', setting_type: 'bool',
    description: 'Enable access to the npm package registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['registry.npmjs.org', '*.npmjs.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.npm.domains', category: 'Package Registries', name: 'npm Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'registry.npmjs.org, *.npmjs.org',
    effective_value: 'registry.npmjs.org, *.npmjs.org',
    enabled_by: 'registry.npm.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'registry.pypi.allow', category: 'Package Registries', name: 'Allow PyPI', setting_type: 'bool',
    description: 'Enable access to the Python Package Index.',
    default_value: true, effective_value: true,
    metadata: { domains: ['pypi.org', 'files.pythonhosted.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.pypi.domains', category: 'Package Registries', name: 'PyPI Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'pypi.org, files.pythonhosted.org',
    effective_value: 'pypi.org, files.pythonhosted.org',
    enabled_by: 'registry.pypi.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'registry.crates.allow', category: 'Package Registries', name: 'Allow crates.io', setting_type: 'bool',
    description: 'Enable access to the Rust crate registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['crates.io', 'static.crates.io'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.crates.domains', category: 'Package Registries', name: 'crates.io Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'crates.io, static.crates.io',
    effective_value: 'crates.io, static.crates.io',
    enabled_by: 'registry.crates.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  // -- Web --
  ms({
    id: 'web.defaults.allow_read', category: 'Web', name: 'Allow read requests', setting_type: 'bool',
    description: 'Allow GET/HEAD/OPTIONS for domains not in any allow/block list.',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'web.defaults.allow_write', category: 'Web', name: 'Allow write requests', setting_type: 'bool',
    description: 'Allow POST/PUT/DELETE/PATCH for domains not in any allow/block list.',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'web.search.google.allow', category: 'Web', name: 'Allow Google', setting_type: 'bool',
    description: 'Enable access to Google web search.',
    default_value: true, effective_value: true,
    metadata: { domains: ['www.google.com', 'google.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'web.search.google.domains', category: 'Web', name: 'Google Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'www.google.com, google.com',
    effective_value: 'www.google.com, google.com',
    enabled_by: 'web.search.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'web.search.bing.allow', category: 'Web', name: 'Allow Bing', setting_type: 'bool',
    description: 'Enable access to Bing web search.',
    default_value: false, effective_value: false,
    metadata: { domains: ['www.bing.com', 'bing.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'web.search.bing.domains', category: 'Web', name: 'Bing Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'www.bing.com, bing.com',
    effective_value: 'www.bing.com, bing.com',
    enabled_by: 'web.search.bing.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'web.search.duckduckgo.allow', category: 'Web', name: 'Allow DuckDuckGo', setting_type: 'bool',
    description: 'Enable access to DuckDuckGo web search.',
    default_value: false, effective_value: false,
    metadata: { domains: ['duckduckgo.com', '*.duckduckgo.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'web.search.duckduckgo.domains', category: 'Web', name: 'DuckDuckGo Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: 'duckduckgo.com, *.duckduckgo.com',
    effective_value: 'duckduckgo.com, *.duckduckgo.com',
    enabled_by: 'web.search.duckduckgo.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'web.custom_allow', category: 'Web', name: 'Allowed domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to allow. Wildcards supported (*.example.com).',
    default_value: 'elie.net, *.elie.net, ash-speed.hetzner.com',
    effective_value: 'elie.net, *.elie.net, ash-speed.hetzner.com',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
  }),
  ms({
    id: 'web.custom_block', category: 'Web', name: 'Blocked domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to block. Takes priority over custom allow list.',
    default_value: '', effective_value: '',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, format: 'domain_list' },
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
  // -- VM > Environment --
  ms({
    id: 'vm.environment.shell.term', category: 'VM', name: 'TERM', setting_type: 'text',
    description: 'Terminal type for the guest shell.',
    default_value: 'xterm-256color', effective_value: 'xterm-256color',
  }),
  ms({
    id: 'vm.environment.shell.home', category: 'VM', name: 'HOME', setting_type: 'text',
    description: 'Home directory for the guest shell.',
    default_value: '/root', effective_value: '/root',
  }),
  ms({
    id: 'vm.environment.shell.path', category: 'VM', name: 'PATH', setting_type: 'text',
    description: 'Executable search path for the guest shell.',
    default_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
    effective_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
  }),
  ms({
    id: 'vm.environment.shell.lang', category: 'VM', name: 'LANG', setting_type: 'text',
    description: 'Locale for the guest shell.',
    default_value: 'C', effective_value: 'C',
  }),
  ms({
    id: 'vm.environment.shell.bashrc', category: 'VM', name: 'Bash configuration', setting_type: 'file',
    description: 'User shell config sourced at login. Customize prompt, aliases, and functions.',
    default_value: { path: '/root/.bashrc', content: '# Prompt: green bold "capsem" with blue directory\nPS1=\'\\[\\033[1;32m\\]capsem\\[\\033[0m\\]:\\[\\033[1;34m\\]\\w\\[\\033[0m\\]\\$ \'\n\n# Aliases\nalias pip=\'uv pip\'\nalias pip3=\'uv pip\'\nalias python=\'uv run python\'\nalias python3=\'uv run python3\'\nalias gemini=\'gemini --yolo\'\nalias ls=\'ls --color=auto\'\nalias ll=\'ls -la --color=auto\'\nalias grep=\'grep --color=auto\'\n' },
    effective_value: { path: '/root/.bashrc', content: '# Prompt: green bold "capsem" with blue directory\nPS1=\'\\[\\033[1;32m\\]capsem\\[\\033[0m\\]:\\[\\033[1;34m\\]\\w\\[\\033[0m\\]\\$ \'\n\n# Aliases\nalias pip=\'uv pip\'\nalias pip3=\'uv pip\'\nalias python=\'uv run python\'\nalias python3=\'uv run python3\'\nalias gemini=\'gemini --yolo\'\nalias ls=\'ls --color=auto\'\nalias ll=\'ls -la --color=auto\'\nalias grep=\'grep --color=auto\'\n' },
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'bash' },
  }),
  ms({
    id: 'vm.environment.shell.tmux_conf', category: 'VM', name: 'tmux configuration', setting_type: 'file',
    description: 'tmux terminal multiplexer config. Customize appearance, keybindings, and behavior.',
    default_value: { path: '/root/.tmux.conf', content: 'set -g default-terminal "tmux-256color"\nset -ag terminal-features ",xterm-256color:RGB"\nset -g mouse on\nset -g escape-time 0\nset -g history-limit 50000\nset -g status-style "bg=default,fg=colour8"\nset -g status-left ""\nset -g status-right ""\nset -g pane-border-style "fg=colour8"\nset -g pane-active-border-style "fg=colour4"\nset -g message-style "bg=default,fg=colour4"\n' },
    effective_value: { path: '/root/.tmux.conf', content: 'set -g default-terminal "tmux-256color"\nset -ag terminal-features ",xterm-256color:RGB"\nset -g mouse on\nset -g escape-time 0\nset -g history-limit 50000\nset -g status-style "bg=default,fg=colour8"\nset -g status-left ""\nset -g status-right ""\nset -g pane-border-style "fg=colour8"\nset -g pane-active-border-style "fg=colour4"\nset -g message-style "bg=default,fg=colour4"\n' },
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, filetype: 'conf' },
  }),
  ms({
    id: 'vm.environment.tls.ca_bundle', category: 'VM', name: 'CA bundle path', setting_type: 'text',
    description: 'Path to the CA certificate bundle in the guest.',
    default_value: '/etc/ssl/certs/ca-certificates.crt',
    effective_value: '/etc/ssl/certs/ca-certificates.crt',
  }),
  // -- VM > Resources --
  ms({
    id: 'vm.resources.cpu_count', category: 'VM', name: 'CPU cores', setting_type: 'number',
    description: 'Number of CPU cores allocated to the VM.',
    default_value: 4, effective_value: 4,
    metadata: { domains: [], choices: [], min: 1, max: 8, rules: {} },
  }),
  ms({
    id: 'vm.resources.ram_gb', category: 'VM', name: 'RAM', setting_type: 'number',
    description: 'Amount of RAM allocated to the VM in GB.',
    default_value: 4, effective_value: 4,
    metadata: { domains: [], choices: [], min: 1, max: 16, rules: {} },
  }),
  ms({
    id: 'vm.resources.scratch_disk_size_gb', category: 'VM', name: 'Scratch disk size', setting_type: 'number',
    description: 'Size of the ephemeral scratch disk in GB.',
    default_value: 16, effective_value: 16,
    metadata: { domains: [], choices: [], min: 1, max: 128, rules: {} },
  }),
  ms({
    id: 'vm.resources.log_bodies', category: 'VM', name: 'Log request bodies', setting_type: 'bool',
    description: 'Capture request/response bodies in telemetry.',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'vm.resources.max_body_capture', category: 'VM', name: 'Max body capture', setting_type: 'number',
    description: 'Maximum bytes of body to capture in telemetry.',
    default_value: 4096, effective_value: 4096,
    metadata: { domains: [], choices: [], min: 0, max: 1048576, rules: {} },
  }),
  ms({
    id: 'vm.resources.retention_days', category: 'VM', name: 'Session retention', setting_type: 'number',
    description: 'Number of days to retain session data.',
    default_value: 30, effective_value: 30,
    metadata: { domains: [], choices: [], min: 1, max: 365, rules: {} },
  }),
  ms({
    id: 'vm.resources.max_sessions', category: 'VM', name: 'Maximum sessions', setting_type: 'number',
    description: 'Keep at most this many sessions (oldest culled first).',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 10000, rules: {} },
  }),
  ms({
    id: 'vm.resources.max_disk_gb', category: 'VM', name: 'Maximum disk usage', setting_type: 'number',
    description: 'Maximum total disk usage for all sessions in GB.',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 1000, rules: {} },
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
          message: `${s.name} not set`,
          docs_url: s.metadata.docs_url ?? null,
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
    kind: 'group', key: 'repository', name: 'Repositories', description: 'Code hosting and git configuration',
    collapsed: false, children: [
      {
        kind: 'group', key: 'repository.git.identity', name: 'Git Identity', description: 'Author name and email for commits inside the VM',
        collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'repository.git.identity.author_name')!),
          leaf(mockSettings.find(s => s.id === 'repository.git.identity.author_email')!),
        ],
      },
      {
        kind: 'group', key: 'repository.providers', name: 'Providers', description: 'Code hosting platforms',
        collapsed: false, children: [
          {
            kind: 'group', key: 'repository.providers.github', name: 'GitHub', description: 'GitHub and GitHub-hosted content',
            enabled_by: 'repository.providers.github.allow', collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'repository.providers.github.allow')!),
              leaf(mockSettings.find(s => s.id === 'repository.providers.github.domains')!),
              leaf(mockSettings.find(s => s.id === 'repository.providers.github.token')!),
            ],
          },
          {
            kind: 'group', key: 'repository.providers.gitlab', name: 'GitLab', description: 'GitLab and GitLab-hosted content',
            enabled_by: 'repository.providers.gitlab.allow', collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'repository.providers.gitlab.allow')!),
              leaf(mockSettings.find(s => s.id === 'repository.providers.gitlab.domains')!),
              leaf(mockSettings.find(s => s.id === 'repository.providers.gitlab.token')!),
            ],
          },
        ],
      },
    ],
  },
  {
    kind: 'group', key: 'registry', name: 'Package Registries', description: 'Package manager registries',
    collapsed: false, children: [
      {
        kind: 'group', key: 'registry.debian', name: 'Debian', description: 'Debian package repositories',
        enabled_by: 'registry.debian.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'registry.debian.allow')!),
          leaf(mockSettings.find(s => s.id === 'registry.debian.domains')!),
        ],
      },
      {
        kind: 'group', key: 'registry.npm', name: 'npm', description: 'npm package registry',
        enabled_by: 'registry.npm.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'registry.npm.allow')!),
          leaf(mockSettings.find(s => s.id === 'registry.npm.domains')!),
        ],
      },
      {
        kind: 'group', key: 'registry.pypi', name: 'PyPI', description: 'Python Package Index',
        enabled_by: 'registry.pypi.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'registry.pypi.allow')!),
          leaf(mockSettings.find(s => s.id === 'registry.pypi.domains')!),
        ],
      },
      {
        kind: 'group', key: 'registry.crates', name: 'crates.io', description: 'Rust crate registry',
        enabled_by: 'registry.crates.allow', collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'registry.crates.allow')!),
          leaf(mockSettings.find(s => s.id === 'registry.crates.domains')!),
        ],
      },
    ],
  },
  {
    kind: 'group', key: 'web', name: 'Web', description: 'Network access control and web services',
    collapsed: false, children: [
      {
        kind: 'group', key: 'web.defaults', name: 'Defaults', description: 'Default actions for unknown domains',
        collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'web.defaults.allow_read')!),
          leaf(mockSettings.find(s => s.id === 'web.defaults.allow_write')!),
        ],
      },
      leaf(mockSettings.find(s => s.id === 'web.custom_allow')!),
      leaf(mockSettings.find(s => s.id === 'web.custom_block')!),
      {
        kind: 'group', key: 'web.search', name: 'Search Engines', description: 'Web search engine access',
        collapsed: false, children: [
          {
            kind: 'group', key: 'web.search.google', name: 'Google', description: 'Google web search',
            enabled_by: 'web.search.google.allow', collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'web.search.google.allow')!),
              leaf(mockSettings.find(s => s.id === 'web.search.google.domains')!),
            ],
          },
          {
            kind: 'group', key: 'web.search.bing', name: 'Bing', description: 'Microsoft Bing web search',
            enabled_by: 'web.search.bing.allow', collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'web.search.bing.allow')!),
              leaf(mockSettings.find(s => s.id === 'web.search.bing.domains')!),
            ],
          },
          {
            kind: 'group', key: 'web.search.duckduckgo', name: 'DuckDuckGo', description: 'DuckDuckGo web search',
            enabled_by: 'web.search.duckduckgo.allow', collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'web.search.duckduckgo.allow')!),
              leaf(mockSettings.find(s => s.id === 'web.search.duckduckgo.domains')!),
            ],
          },
        ],
      },
    ],
  },
  {
    kind: 'group', key: 'appearance', name: 'Appearance', description: 'UI appearance and display settings',
    collapsed: false, children: [
      leaf(mockSettings.find(s => s.id === 'appearance.dark_mode')!),
      leaf(mockSettings.find(s => s.id === 'appearance.font_size')!),
    ],
  },
  {
    kind: 'group', key: 'vm', name: 'VM', description: 'Virtual machine configuration',
    collapsed: false, children: [
      {
        kind: 'group', key: 'vm.environment', name: 'Environment', description: 'Shell and environment variables',
        collapsed: false, children: [
          {
            kind: 'group', key: 'vm.environment.shell', name: 'Shell', description: 'Guest shell settings',
            collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.term')!),
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.home')!),
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.path')!),
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.lang')!),
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.bashrc')!),
              leaf(mockSettings.find(s => s.id === 'vm.environment.shell.tmux_conf')!),
            ],
          },
          {
            kind: 'group', key: 'vm.environment.tls', name: 'TLS', description: 'TLS certificate configuration',
            collapsed: false, children: [
              leaf(mockSettings.find(s => s.id === 'vm.environment.tls.ca_bundle')!),
            ],
          },
        ],
      },
      {
        kind: 'group', key: 'vm.resources', name: 'Resources', description: 'Hardware, telemetry, and session limits',
        collapsed: false, children: [
          leaf(mockSettings.find(s => s.id === 'vm.resources.cpu_count')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.ram_gb')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.scratch_disk_size_gb')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.log_bodies')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.max_body_capture')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.retention_days')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.max_sessions')!),
          leaf(mockSettings.find(s => s.id === 'vm.resources.max_disk_gb')!),
        ],
      },
    ],
  },
  ];
}

// ---------------------------------------------------------------------------
// MCP mock data
// ---------------------------------------------------------------------------

const MOCK_MCP_SERVERS: McpServerInfo[] = [
  {
    name: 'github',
    command: 'npx',
    args: ['-y', '@github/mcp-server'],
    source: 'claude',
    enabled: true,
    running: true,
    tool_count: 4,
  },
  {
    name: 'slack',
    command: 'npx',
    args: ['-y', '@slack/mcp-server'],
    source: 'gemini',
    enabled: true,
    running: true,
    tool_count: 3,
  },
  {
    name: 'internal-tools',
    command: '/usr/local/bin/corp-mcp',
    args: ['--config', '/etc/corp/mcp.json'],
    source: 'manual',
    enabled: false,
    running: false,
    tool_count: 0,
  },
];

const MOCK_MCP_TOOLS: McpToolInfo[] = [
  {
    namespaced_name: 'github__search_repos',
    original_name: 'search_repos',
    description: 'Search GitHub repositories by query string',
    server_name: 'github',
    annotations: { title: 'Search Repos', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: 'a1b2c3',
    approved: true,
    pin_changed: false,
  },
  {
    namespaced_name: 'github__create_issue',
    original_name: 'create_issue',
    description: 'Create a new issue on a repository',
    server_name: 'github',
    annotations: { title: 'Create Issue', read_only_hint: false, destructive_hint: false, idempotent_hint: false, open_world_hint: true },
    pin_hash: 'd4e5f6',
    approved: true,
    pin_changed: false,
  },
  {
    namespaced_name: 'github__delete_repo',
    original_name: 'delete_repo',
    description: 'Delete a repository (destructive)',
    server_name: 'github',
    annotations: { title: 'Delete Repo', read_only_hint: false, destructive_hint: true, idempotent_hint: false, open_world_hint: true },
    pin_hash: 'g7h8i9',
    approved: false,
    pin_changed: false,
  },
  {
    namespaced_name: 'github__list_prs',
    original_name: 'list_prs',
    description: 'List pull requests on a repository',
    server_name: 'github',
    annotations: { title: 'List PRs', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: 'j1k2l3',
    approved: true,
    pin_changed: false,
  },
  {
    namespaced_name: 'slack__send_message',
    original_name: 'send_message',
    description: 'Send a message to a Slack channel',
    server_name: 'slack',
    annotations: { title: 'Send Message', read_only_hint: false, destructive_hint: false, idempotent_hint: false, open_world_hint: true },
    pin_hash: 'm4n5o6',
    approved: true,
    pin_changed: false,
  },
  {
    namespaced_name: 'slack__list_channels',
    original_name: 'list_channels',
    description: 'List available Slack channels',
    server_name: 'slack',
    annotations: { title: 'List Channels', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false },
    pin_hash: 'p7q8r9',
    approved: true,
    pin_changed: false,
  },
  {
    namespaced_name: 'slack__upload_file',
    original_name: 'upload_file',
    description: 'Upload a file to a Slack channel',
    server_name: 'slack',
    annotations: null,
    pin_hash: 's1t2u3',
    approved: false,
    pin_changed: true,
  },
  {
    namespaced_name: 'github__read_file',
    original_name: 'read_file',
    description: 'Read a file from a repository (definition changed)',
    server_name: 'github',
    annotations: { title: 'Read File', read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true },
    pin_hash: 'changed_hash',
    approved: false,
    pin_changed: true,
  },
];

const MOCK_MCP_POLICY: McpPolicyInfo = {
  global_policy: 'allow',
  default_tool_permission: 'allow',
  blocked_servers: [],
  tool_permissions: {
    'github__delete_repo': 'block',
  },
};

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
      'deb.debian.org', 'security.debian.org',
      'registry.npmjs.org', '*.npmjs.org',
      'pypi.org', 'files.pythonhosted.org',
      'crates.io', 'static.crates.io',
      '*.googleapis.com',
      'www.google.com', 'google.com',
      'elie.net', '*.elie.net', 'ash-speed.hetzner.com',
    ],
    block: [
      '*.anthropic.com', '*.claude.com',
      '*.openai.com',
      'www.bing.com', 'bing.com',
      'duckduckgo.com', '*.duckduckgo.com',
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
  getMcpServers: async (): Promise<McpServerInfo[]> => MOCK_MCP_SERVERS.map(s => ({ ...s })),
  getMcpTools: async (): Promise<McpToolInfo[]> => MOCK_MCP_TOOLS.map(t => ({ ...t })),
  getMcpPolicy: async (): Promise<McpPolicyInfo> => ({ ...MOCK_MCP_POLICY, tool_permissions: { ...MOCK_MCP_POLICY.tool_permissions } }),
  setMcpServerEnabled: async (_name: string, _enabled: boolean) => {},
  addMcpServer: async (_name: string, _command: string, _args: string[], _env: Record<string, string>) => {},
  removeMcpServer: async (_name: string) => {},
  setMcpGlobalPolicy: async (_policy: string) => {},
  setMcpDefaultPermission: async (_permission: string) => {},
  setMcpToolPermission: async (_tool: string, _permission: string) => {},
  approveMcpTool: async (_tool: string) => {},
  refreshMcpTools: async (_server?: string) => {},
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
  onDownloadProgress: async (cb: (progress: any) => void) => {
    // Simulate download progress for UI preview
    let bytes = 0;
    const total = 437 * 1024 * 1024; // 437 MB
    const step = total / 50;
    cb({ asset: 'rootfs.squashfs', bytes_downloaded: 0, total_bytes: total, phase: 'connecting' });
    const iv = setInterval(() => {
      bytes = Math.min(bytes + step, total);
      cb({ asset: 'rootfs.squashfs', bytes_downloaded: bytes, total_bytes: total, phase: bytes >= total ? 'verifying' : 'downloading' });
      if (bytes >= total) clearInterval(iv);
    }, 400);
    return () => clearInterval(iv);
  },
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
