// Marketing copy and reviewed documentation destinations live here.
export const SITE = {
  name: "Capsem",
  tagline: "The fastest way to ship with AI securely.",
  description: "A secure environment for your AI agents. Your tools, connections, and workspace, together.",
  installCmd: "curl -fsSL https://capsem.org/install.sh | sh",
  github: "https://github.com/google/capsem",
  docs: "https://docs.capsem.org",
  releases: "https://release.capsem.org/channels/stable/",
  platform: "macOS · Apple Silicon / Linux · x86_64 & ARM64",
  license: "Open source · MIT licensed",
} as const;
export const DOCS = {
  start: `${SITE.docs}/getting-started/installation/`,
  use: `${SITE.docs}/getting-started/how-to-use-capsem/`,
  images: `${SITE.docs}/guides/enterprise/build-an-image/`,
  security: `${SITE.docs}/architecture/security/`,
  enterprise: `${SITE.docs}/guides/enterprise/deploy-capsem/`,
  activity: `${SITE.docs}/guides/enterprise/investigate-a-vm/`,
  sdk: `${SITE.docs}/sdk/`,
} as const;
export const NAV_LINKS = [
  { label: "Get started", href: DOCS.start },
  { label: "Security", href: DOCS.security },
  { label: "Enterprise", href: DOCS.enterprise },
  { label: "SDK", href: DOCS.sdk },
] as const;
export const UI = {
  download: "Download", source: "GitHub", copy: "Copy command", copied: "Copied", copyFailed: "Select and copy the command",
  openMenu: "Open menu", closeMenu: "Close menu", skip: "Skip to content",
} as const;
export const HERO = {
  lines: ["The fastest way", "to ship with AI", "securely."],
  title: "Get more done with AI.",
  description: "Your agents, tools, and connections. Set up for you. Isolated by design.",
} as const;
export const DEMO = {
  title: "Capsem", session: "inbox", badge: "Isolated VM", label: "Illustrated workflow", play: "Play demo", pause: "Pause demo", replay: "Replay demo",
  steps: ["Launch", "Connect", "Get it done"],
  agent: "OpenAI Codex", directory: "directory", directoryValue: "/workspace",
  tabs: ["Terminal", "Stats", "Files"], prompt: "› Ask Codex to do anything", hint: "? for shortcuts", context: "100% context left",
  lines: [
    { text: "› Find the emails that need my attention.", kind: "prompt" },
    { text: "• Reading your connected inbox", kind: "muted" },
    { text: "• Here are the three emails to focus on:", kind: "result" },
    { text: "1. Project proposal — review requested", kind: "item" },
    { text: "2. Tomorrow’s meeting — confirm attendance", kind: "item" },
    { text: "3. Updated timeline — approval needed", kind: "item" },
  ],
  connection: "Email connected", boundary: "Protected by Capsem",
} as const;
export const AGENTS = {
  title: "Your favorite AIs.",
  description: "Switch anytime.",
  names: ["Claude Code", "Codex", "Gemini CLI", "Antigravity", "Ollama"],
  toolsTitle: "Everything you need, out of the box.",
  groups: [
    { name: "Code", items: ["Python", "Node.js", "NumPy", "pandas", "SciPy", "scikit-learn", "Matplotlib", "pytest"] },
    { name: "Tools", items: ["Git", "GitHub CLI", "uv", "tmux", "curl", "FastMCP"] },
    { name: "MCP tools", items: ["fetch_http", "grep_http", "http_headers"] },
  ],
  custom: "Make it yours", detail: "Your tools. Your MCP servers. Your own images.", link: "Build your environment",
} as const;
export const SPEED = {
  title: "Built to help you move fast.",
  description: "Nothing slows you down.",
  stats: [
    { value: "1.13", unit: "s", label: "Provision a VM" },
    { value: "28.5", unit: "ms", label: "Execute a command" },
    { value: "55.9", unit: "ms", label: "Fork an environment" },
  ],
  note: "Recorded macOS ARM64 benchmarks · June 2026", methodology: "Measurement details",
  details: "Means of 3 runs from the checked-in macOS ARM64 baseline. Provisioning, command execution, and forking are separate operations; these are not end-to-end agent response times. Results vary by hardware and workload.",
  sources: [
    { label: "Lifecycle data", href: `${SITE.github}/blob/main/benchmarks/baselines/lifecycle/data_1.3.1782571508.json` },
    { label: "Fork data", href: `${SITE.github}/blob/main/benchmarks/baselines/fork/data_1.3.1782571508.json` },
  ],
} as const;
export const SECURITY = {
  title: "Secure by default", subtitle: "so you can focus on your goal.",
  description: "Isolation, protected credentials, and checks on what goes in and out.", link: "Explore the security architecture",
  diagram: "An isolated agent workspace, hosted in a low-privilege process, connects to models and tools through Capsem’s security engine. Credentials stay outside the VM. Rules, privacy scanning, logging and reporting apply at the security boundary.",
  process: "Low-privilege process", workspace: "Your workspace", agent: "AI agent", tools: "Tools & files", isolation: "Isolated Linux VM", airgap: "No direct network access",
  engine: "Security engine", controls: ["Access rules", "Privacy scanning", "Credential interposition"],
  credentials: "Credentials", outside: "Kept outside the VM", destinations: ["AI models", "MCP tools", "Web & APIs"],
  logging: "Logging & reporting", trail: "Activity. Decisions. Evidence.",
} as const;
export const CONTROL = {
  title: "Stay in control.",
  caption: "Illustrative examples",
  panels: [
    { title: "See everything.", description: "Every action, request, and decision is recorded.", label: "Interactions", kind: "activity" },
    { title: "Full control.", description: "Seamlessly set boundaries at the semantic level.", label: "CEL rules", kind: "rules" },
    { title: "Detect threats.", description: "Flag access to password files and secrets.", label: "Sigma detections", kind: "detection" },
  ],
  viewer: { type: "Tool call", time: "09:41:03", trace: "trace a8f2 · turn 1", tool: "fetch_http", server: "capsem", decision: "allow", arguments: "Arguments", request: '{"url":"https://example.com"}', response: "Observed response", result: "200 OK · text/html · 2.4 KB", next: "Model response", recorded: "Request · Response · Decision" },
  rules: [
    { name: "Restrict MCP tools", expression: 'mcp.tool_call.name == "delete_file"', action: "deny", icon: "◇" },
    { name: "Choose agent models", expression: 'model.provider == "openai"', action: "allow", icon: "✦" },
    { name: "Restrict websites", expression: 'http.host == "untrusted.example"', action: "deny", icon: "↗" },
  ],
  detection: 'title: Password file accessed\nlevel: high\ndetection:\n  selection:\n    file.read.name: passwords.txt\n  condition: selection',
  alert: "Password file accessed", severity: "High", evidence: "file.read.name = passwords.txt",
  links: [{ label: "Explore observability", href: DOCS.activity }, { label: "Deploy for your team", href: DOCS.enterprise }],
} as const;
