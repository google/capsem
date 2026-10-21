import type { Diagram } from '@capsem/diagrams/types';

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
  { label: "Getting started", href: `${SITE.docs}/getting-started/intro/` },
  { label: "Guides", href: `${SITE.docs}/guides/` },
  { label: "Architecture", href: `${SITE.docs}/architecture/overview/` },
  { label: "SDK", href: DOCS.sdk },
  { label: "Reference", href: `${SITE.docs}/reference/` },
] as const;
export const DOWNLOAD = {
  title: "Download Capsem", other: "Other downloads", close: "Close downloads",
  choose: "Choose your platform",
  manifest: "https://release.capsem.org/assets/stable/manifest.json",
  packages: [
    { platform: "macos", architecture: "arm64", label: "macOS", detail: "Apple Silicon · .pkg", url: "https://github.com/google/capsem/releases/download/v0.6.6/Capsem-0.6.6.pkg" },
    { platform: "linux", architecture: "amd64", label: "Linux", detail: "x86_64 · .deb", url: "https://github.com/google/capsem/releases/download/v0.6.6/Capsem_0.6.6_amd64.deb" },
    { platform: "linux", architecture: "arm64", label: "Linux", detail: "ARM64 · .deb", url: "https://github.com/google/capsem/releases/download/v0.6.6/Capsem_0.6.6_arm64.deb" },
  ],
} as const;
export const UI = {
  download: "Download", source: "GitHub", copy: "Copy command", copied: "Copied", copyFailed: "Select and copy the command",
  openMenu: "Open menu", closeMenu: "Close menu", skip: "Skip to content",
} as const;
export const AGENTS = {
  title: "Your favorite AIs.",
  description: "Switch anytime — or use all of them in parallel.",
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
    { value: "1.2", unit: "s", label: "Launch an environment" },
    { value: "3k", unit: "req/s", label: "Network that keeps up" },
    { value: "60", unit: "ms", label: "Duplicate your environment" },
  ],
  note: "Recorded benchmarks · May–June 2026", methodology: "Measurement details",
  details: "Environment launch and duplication: macOS ARM64, June 2026, means of 3 runs (1,132.2 ms and 55.9 ms), rounded conservatively to 1.2 s and 60 ms. Network: historical May 2026 proxy baseline, 3,042.6 requests/s at 10 concurrent requests over 10 seconds. These isolated benchmarks do not measure agent readiness or external network speed. Results vary by hardware and workload.",
  sources: [
    { label: "Lifecycle data", href: `${SITE.github}/blob/main/benchmarks/baselines/lifecycle/data_1.3.1782571508.json` },
    { label: "Network data", href: `${SITE.github}/blob/main/benchmarks/baselines/mitm-load/baseline.json` },
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
export const SECURITY_DIAGRAM: Diagram = {
  title: SECURITY.title, description: SECURITY.diagram, width: 1080, height: 590,
  nodes: [
    { id: 'process', label: SECURITY.process, x: 20, y: 85, width: 310, height: 445, boundary: 'group' },
    { id: 'vm', label: SECURITY.isolation, x: 40, y: 145, width: 270, height: 335, boundary: 'vm' },
    { id: 'workspace', label: `${SECURITY.workspace}\n${SECURITY.agent}\n${SECURITY.tools}\n\n${SECURITY.airgap}`, x: 60, y: 215, width: 230, height: 180 },
    { id: 'engine', label: `${SECURITY.engine}\n\n${SECURITY.controls.join('\n')}`, x: 415, y: 215, width: 290, height: 180, tone: 'processing' },
    { id: 'credentials', label: `${SECURITY.credentials}\n${SECURITY.outside}`, x: 435, y: 30, width: 250, tone: 'credential' },
    ...SECURITY.destinations.map((label, i) => ({ id: `destination-${i}`, label, x: 830, y: 170 + i * 110, width: 220 })),
    { id: 'logging', label: `${SECURITY.logging}\n${SECURITY.trail}`, x: 415, y: 490, width: 290 },
  ],
  links: [
    { from: 'vm', to: 'engine', label: '', source: 'right', target: 'left' },
    { from: 'credentials', to: 'engine', label: '', control: true },
    ...SECURITY.destinations.map((_, i) => ({ from: 'engine', to: `destination-${i}`, label: '', source: 'right' as const, target: 'left' as const, via: [{ x: 770, y: 305 }, { x: 770, y: 206 + i * 110 }] })),
    { from: 'engine', to: 'logging', label: '', control: true },
  ],
};
export const CONTROL = {
  title: "Stay in control.",
  caption: "Illustrative examples",
  panels: [
    { title: "See everything.", description: "Every action, request, and decision is recorded.", label: "Interactions", kind: "activity" },
    { title: "Full control.", description: "Seamlessly set boundaries at the semantic level.", label: "CEL rules", kind: "rules" },
    { title: "Detect threats.", description: "Flag access to password files and secrets.", label: "How to write a detection rule", href: `${SITE.docs}/guides/enterprise/write-detection-rules/`, kind: "detection" },
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

export const HERO_DEMO = {
  terminalLabel: "Capsem terminal demonstration",
  descriptionForScreenReaders: "A continuous desktop demonstration: open Terminal from the dock and install Capsem; create an AGY VM, open the Capsem TUI and build a website; create a new session named Email productivity inside the TUI and ask Codex to review fictional emails; open Capsem's GUI to see parallel agents, then return to the TUI and ask your agent through MCP. Each chapter opens with a marketing interstitial.",
  pause: "Pause animation", play: "Play animation", replay: "Replay animation",
  chaptersLabel: "Demo chapters",
  chapters: ["Install", "Ship fast", "Swap workflow", "Scale"],
  desktop: { terminal: "Terminal", browser: "Browser", app: "Capsem", menus: ["File", "Edit", "View"], clock: "Mon 9:41", },
  inserts: {
    install: { eyebrow: "UP AND RUNNING", title: "Easy to install.", detail: "One command. Everything you need, ready to go." },
    build: { eyebrow: "FROM IDEA TO WEBSITE", title: "Ship fast.", detail: "Your favorite agent and tools are all ready to go." },
    work: { eyebrow: "NEW TASK — NEW AGENT", title: "Swap workflow.", detail: "A new workflow. A fresh VM. Get things done." },
    scale: { eyebrow: "MORE AGENTS. MORE DONE.", title: "Scale.", detail: "Work in parallel. TUI, GUI, API — or just ask your agent." },
  },
  tui: {
    name: "capsem", websiteTitle: "Website", email: "Email productivity",
    sessions: "sessions", create: "new session", nameLabel: "name", profiles: "profiles",
    inputHint: "active input: name; type to edit; Backspace deletes",
    createHint: "Up/Down selects profile; Enter creates; Esc cancels",
    profileColumns: ["Pick", "Profile", "Name"],
    profileOptions: [{ id: "code", name: "Code" }, { id: "co-work", name: "Co-work" }],
    columns: ["#", "Name", "Profile", "State", "Time", "Tokens", "Cost"],
    help: "help: alt+?",
  },
  scale: {
    label: "Capsem managing parallel agent sessions", controlsLabel: "Control Capsem your way",
    tab: "Sessions", refresh: "Refresh", running: "Running", booting: "Booting",
    columns: ["Name", "Status", "Profile", "Uptime", "Tokens", "Cost", "Actions"],
    surfaces: [{ id: "gui", label: "GUI" }, { id: "tui", label: "TUI" }, { id: "api", label: "API" }, { id: "mcp", label: "Ask your agent · MCP" }],
    views: ["Terminal", "Stats", "Files"],
    terminal: "     ▄▄\n    ▄██▄\n   ▄█▀▀█▄\n  ▄█▀  ▀█▄\n ▄█▀    ▀█▄\n\nAntigravity CLI\nGemini · ~/website\n\n> Build a website for a design studio.\n\n● Created index.html (+84)\n● Created styles.css (+126)\n● Started preview server\n\nReady at http://localhost:3000\n\n> ",
    mcp: { prompt: "Check on all my agents.", call: "Called capsem.capsem_list({})", result: "All 6 agents are running in separate VMs.", detail: "Website + Email productivity.\nAPI, tests, docs and review.\nAll moving in parallel." },
    sessions: [
      { name: "Website", profile: "code", uptime: "24s", tokens: "8.2K", cost: "$0.04" },
      { name: "Email productivity", profile: "co-work", uptime: "21s", tokens: "3.1K", cost: "$0.02" },
      { name: "API", profile: "code", uptime: "18s", tokens: "6.4K", cost: "$0.03" },
      { name: "Tests", profile: "code", uptime: "16s", tokens: "2.8K", cost: "$0.01" },
      { name: "Docs", profile: "code", uptime: "12s", tokens: "4.6K", cost: "$0.02" },
      { name: "Review", profile: "code", uptime: "8s", tokens: "1.9K", cost: "$0.01" },
    ],
  },
  codex: {
    name: "OpenAI Codex", modelLabel: "model:", model: "gpt-6.1-sol medium",
    directoryLabel: "directory:", directory: "~/inbox", changeModel: "/model to change",
    placeholder: "Ask Codex to do anything", context: "100% context left · ? for shortcuts",
    task: "Summarize my emails.",
    welcome: "To get started, describe a task.\r\n\r\n/model        Choose a model\r\n/permissions  Set permissions",
  },
  shellTitle: "elie — zsh",
  previewLabel: "Website preview", previewURL: "localhost:3000",
  preview: { brand: "FORM & FIELD", nav: "Independent design studio", eyebrow: "THOUGHTFUL BY DESIGN", title: "A little space.\nA bigger idea.", description: "We create places, objects, and experiences that make everyday life a little more extraordinary.", action: "Explore our work ↗", footer: "Architecture / Objects / Digital", monogram: "F/f" },
} as const;

// Fictional transcript, rendered as terminal output; no remote commands run.
const dim = (text: string) => `\x1b[38;5;245m${text}\x1b[0m`;
const blue = (text: string) => `\x1b[38;5;111m${text}\x1b[0m`;
const bold = (text: string) => `\x1b[1m${text}\x1b[0m`;
const prompt = `\r\n${blue("❯")} `;
const agyLogo = [
  "     ▄▄", "    ▄██▄", "   ▄█▀▀█▄", "  ▄█▀  ▀█▄", " ▄█▀    ▀█▄",
].map((line, index) => `\x1b[38;5;${[209, 221, 177, 141, 111][index]}m${line}\x1b[0m`).join("\r\n");
export const HERO_TERMINAL_SCRIPT: import("./hero-terminal").TerminalStep[] = [
  { chapter: 0, insert: "install", window: false, cursor: "start", wait: 1700 },
  { insert: null, cursor: "terminal", wait: 650 },
  { dock: "terminal", click: true, wait: 220 },
  { window: true, dock: null, click: false, cursor: "body", write: prompt, wait: 500 },
  { type: SITE.installCmd, wait: 300 },
  { write: `\r\n${dim("Downloading Capsem")}\r\n`, wait: 350 },
  { write: `${blue("  ███████░░░░░░░░░░░░░  32%")}`, wait: 350 },
  { write: `\r\x1b[2K${blue("  █████████████░░░░░░░  64%")}`, wait: 350 },
  { write: `\r\x1b[2K${blue("  ████████████████████ 100%")}\r\n`, wait: 300 },
  { write: `${dim("Installing CLI…")}\r\n${blue("✓ Capsem installed")}\r\n`, wait: 1000 },
  { chapter: 1, insert: "build", wait: 1700 },
  { insert: null, write: prompt, type: "capsem create --image agy -n Website", wait: 350 },
  { write: `\r\n${dim("Pulling agy…")}\r\n${dim("Booting VM…")}\r\n`, wait: 850 },
  { write: `${blue("Website (persistent)")}\r\n${prompt}`, type: "capsem shell", wait: 500 },
  { tui: true, overlay: "sessions", workflow: "website", surface: "tui", title: HERO_DEMO.tui.name, clear: true, write: `\r\n${agyLogo}\r\n\r\n${bold("Antigravity CLI")}\r\n${dim("Gemini · ~/website")}\r\n\r\n> `, wait: 1100 },
  { overlay: null, key: "Enter", wait: 350 },
  { key: null, type: "Build a website for a design studio.", wait: 650 },
  { write: `\r\n${dim("──────────────────────────────────")}\r\n\r\n${blue("⠋")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("⠹")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("●")} Created index.html ${dim("(+84)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Created styles.css ${dim("(+126)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Started preview server\r\n\r\nReady at ${blue("http://localhost:3000")}\r\n`, wait: 700 },
  { cursor: "link", wait: 650 },
  { click: true, wait: 180 },
  { preview: true, click: false, cursor: "preview", wait: 2500 },
  { preview: false, cursor: "body", wait: 300 },
  { chapter: 2, insert: "work", wait: 1700 },
  { insert: null, overlay: "create", key: "⌥ N", profile: "code", input: true, type: HERO_DEMO.tui.email, wait: 700 },
  { profile: "co-work", key: "↓", wait: 500 },
  { overlay: null, key: "Enter", email: true, workflow: "email", clear: true, write: `${dim("Starting Email productivity…")}\r\n`, wait: 850 },
  { key: null, write: prompt, type: "codex", wait: 500 },
  { codex: true, clear: true, write: `\x1b[?25l${dim(HERO_DEMO.codex.welcome)}`, wait: 700 },
  { input: true, type: HERO_DEMO.codex.task, wait: 450 },
  { clear: true, write: `› ${HERO_DEMO.codex.task}\r\n\r\n${dim("• Reading your inbox…")}`, wait: 750 },
  { write: `\r\x1b[2K${dim("• Called mail.list_messages")}\r\n${dim("  └ 3 messages returned")}\r\n\r\n`, wait: 450 },
  { write: `${bold("Your inbox, summarized:")}\r\n\r\n`, wait: 300 },
  { write: `• ${bold("Maya")} — website ready for review.\r\n  ${dim("Send feedback today.")}\r\n`, wait: 550 },
  { write: `• ${bold("Design team")} — new concepts ready.\r\n  ${dim("Pick a direction by Friday.")}\r\n`, wait: 550 },
  { write: `• ${bold("Alex")} — coffee on Thursday?\r\n  ${dim("Confirm a time.")}\r\n`, wait: 700 },
  { chapter: 3, insert: "scale", wait: 1700 },
  { insert: null, cursor: "app", wait: 650 },
  { dock: "app", click: true, wait: 220 },
  { surface: "gui", dock: null, click: false, cursor: "body", scale: 2, wait: 850 },
  { scale: 4, wait: 650 },
  { scale: HERO_DEMO.scale.sessions.length, wait: 1700 },
  { cursor: "session", wait: 650 },
  { click: true, wait: 180 },
  { guiTerminal: true, click: false, cursor: "body", wait: 1700 },
  { cursor: "terminal", wait: 650 },
  { dock: "terminal", click: true, wait: 220 },
  { surface: "tui", overlay: "sessions", codex: false, dock: null, click: false, cursor: "body", wait: 1800 },
  { overlay: null, key: "⌥ 2", codex: true, wait: 500 },
  { surface: "mcp", key: null, input: true, type: HERO_DEMO.scale.mcp.prompt, wait: 450 },
  { clear: true, write: `› ${HERO_DEMO.scale.mcp.prompt}\r\n\r\n${dim("• " + HERO_DEMO.scale.mcp.call)}\r\n${dim("  └ 6 sessions returned")}\r\n\r\n${bold(HERO_DEMO.scale.mcp.result)}\r\n\r\n${HERO_DEMO.scale.mcp.detail}`, wait: 3200 },
];
