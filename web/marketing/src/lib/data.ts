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
  title: "Your workspace", session: "inbox", badge: "Isolated VM", label: "Illustrated workflow", play: "Play demo", pause: "Pause demo", replay: "Replay demo",
  steps: ["Launch", "Connect", "Get it done"],
  lines: [
    { text: "$ capsem create -n inbox", kind: "command" },
    { text: "✓ Workspace created · security active", kind: "status" },
    { text: "$ capsem shell", kind: "command" },
    { text: "Your agent is ready.", kind: "status" },
    { text: "› Find the emails that need my attention.", kind: "prompt" },
    { text: "Reading your connected inbox…", kind: "muted" },
    { text: "3 things to focus on today", kind: "result" },
    { text: "01  Review the project proposal", kind: "item" },
    { text: "02  Confirm tomorrow’s meeting", kind: "item" },
    { text: "03  Approve the updated timeline", kind: "item" },
  ],
  connection: "Email connected", boundary: "Protected by Capsem",
} as const;
export const AGENTS = {
  title: "Your AI. Your environment.",
  description: "Use your favorites. Switch freely. Build your own.",
  names: ["Claude Code", "Codex", "Gemini CLI", "Antigravity", "Ollama"],
  custom: "Make it yours", detail: "Your tools. Your MCP servers. Your own images.", link: "Build your environment",
} as const;
export const SPEED = {
  title: "Built to help you move fast.",
  description: "Lightweight VMs. Native virtualization. More time for your work.",
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
  title: "Secure by default.", subtitle: "So you can focus on your goal.",
  description: "Isolation, protected credentials, and checks on what goes in and out.", link: "Explore the security architecture",
  diagram: "An isolated agent workspace connects to models and tools through Capsem’s security engine. Credentials stay outside the VM. Rules, privacy scanning, logging and reporting apply at the security boundary.",
  workspace: "Your workspace", agent: "AI agent", tools: "Tools & files", isolation: "Isolated Linux VM", airgap: "No direct network access",
  engine: "Security engine", controls: ["Access rules", "Privacy scanning", "Credential interposition"],
  credentials: "Credentials", outside: "Kept outside the VM", destinations: ["AI models", "MCP tools", "Web & APIs"],
  logging: "Logging & reporting", trail: "Activity. Decisions. Evidence.",
} as const;
export const CONTROL = {
  title: "Stay in control.",
  description: "See what your agents do. Set the rules for your team.",
  links: [{ label: "Explore observability", href: DOCS.activity }, { label: "Deploy for your team", href: DOCS.enterprise }],
  titlebar: "Session activity", caption: "Illustrative session", session: "inbox", status: "Running",
  columns: ["Time", "Activity", "Decision"],
  rows: [
    { time: "09:41:02", title: "Model request", detail: "Credential applied by broker", status: "Allowed", denied: false },
    { time: "09:41:03", title: "Read inbox", detail: "MCP · email connector", status: "Allowed", denied: false },
    { time: "09:41:04", title: "Outbound request", detail: "Destination outside policy", status: "Blocked", denied: true },
  ],
  policy: "Your policy. Applied consistently.", policyDetail: "Enforcement · Detections · Reporting",
} as const;
