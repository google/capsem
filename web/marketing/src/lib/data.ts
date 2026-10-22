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
export const SETUP = {
  title: "Set up with your AI",
  openIn: "Open in",
  copy: "Copy setup prompt",
  copied: "Prompt copied",
  failed: "Select and copy the prompt below.",
  show: "View setup prompt",
  paste: "Paste into your agent to get started.",
  guide: "Installation guide",
  agents: [
    { name: "Claude Code", icon: "claude" },
    { name: "Codex", icon: "openai" },
    { name: "Cursor", icon: null },
  ],
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
  descriptionForScreenReaders: "Install Capsem, build and preview a website in an AGY VM, explicitly create a separate Email productivity VM through the TUI, then open the GUI and create four more sessions. Type a different prompt in each tab while earlier agents keep working independently in their own VMs.",
  pause: "Pause animation", play: "Play animation", replay: "Replay animation",
  chaptersLabel: "Demo chapters",
  chapters: ["Install", "Ship fast", "Swap workflow", "Scale"],
  desktop: { terminal: "Terminal", browser: "Browser", app: "Capsem", menus: ["File", "Edit", "View"], clock: "Mon 9:41", },
  inserts: {
    install: { eyebrow: "UP AND RUNNING", title: "Easy to install.", detail: "One command. Ready to work." },
    build: { eyebrow: "FROM IDEA TO WEBSITE", title: "Ship fast.", detail: "Your favorite agent and tools are all ready to go." },
    work: { eyebrow: "NEW TASK — NEW VM", title: "Swap workflow.", detail: "Create a session. Name your task. Start your agent." },
    scale: { eyebrow: "MORE AGENTS. MORE DONE.", title: "Work in parallel.", detail: "More sessions. More prompts. Every task in its own VM." },
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
    label: "Capsem managing parallel agent sessions",
    tab: "Sessions", refresh: "Refresh", running: "Running", booting: "Booting",
    columns: ["Name", "Status", "Profile", "Uptime", "Tokens", "Cost", "Actions"],
    views: ["Terminal", "Stats", "Files"],
    start: "Start from a profile", profileName: "Code", profileDescription: "Optimized for coding and long-running agents.",
    new: "+ New", customize: "Customize", dialog: "Customize session", create: "Create", cancel: "Cancel",
    profileLabel: "Profile", nameLabel: "Name", ramLabel: "RAM (MB)", cpuLabel: "CPUs",
    boot: "Booting a new VM…", waiting: "Starting agent…", ready: "Session ready", working: "Working…",
    terminalLabel: "Agent terminal", parallelCaption: "Six prompts running. Six separate VMs.",
    websiteReady: "● Website ready at http://localhost:3000\r\n\r\n> ", emailReady: "Your inbox, summarized.\r\n3 messages reviewed.\r\n",
    sessions: [
      { name: "Website", profile: "code", uptime: "24s", tokens: "8.2K", cost: "$0.04", prompt: "Polish the mobile layout.", output: ["Reading styles.css", "Adjusting responsive spacing", "Checking the mobile preview", "Refining typography", "Verifying responsive layouts"] },
      { name: "Email productivity", profile: "co-work", uptime: "21s", tokens: "3.1K", cost: "$0.02", prompt: "Draft replies to the action items.", output: ["Reading the inbox summary", "Drafting a reply to Maya", "Preparing the design feedback", "Drafting a reply to Alex", "Reviewing the drafts"] },
      { name: "API", profile: "code", uptime: "0s", tokens: "0", cost: "$0.00", prompt: "Build the contact API.", output: ["Reading the app structure", "Creating the contact endpoint", "Adding input validation", "Writing endpoint tests", "Running the API tests"] },
      { name: "Tests", profile: "code", uptime: "0s", tokens: "0", cost: "$0.00", prompt: "Test the signup flow.", output: ["Inspecting the signup form", "Adding validation tests", "Testing success and failure paths", "Running browser tests", "Checking edge cases"] },
      { name: "Docs", profile: "code", uptime: "0s", tokens: "0", cost: "$0.00", prompt: "Write the getting-started guide.", output: ["Reading the README", "Documenting setup", "Adding usage examples", "Checking the example commands", "Reviewing the guide"] },
      { name: "Review", profile: "code", uptime: "0s", tokens: "0", cost: "$0.00", prompt: "Review auth for security issues.", output: ["Reading authentication handlers", "Tracing the session lifecycle", "Checking token validation", "Reviewing access controls", "Checking authorization paths"] },
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
// Both TUI and GUI use this exact ANSI banner through xterm.
export const HERO_AGY_BANNER = `${agyLogo}\r\n\r\n${bold("Antigravity CLI")}\r\n${dim("Gemini · ~/website")}`;
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
  { tui: true, overlay: "sessions", workflow: "website", surface: "tui", title: HERO_DEMO.tui.name, clear: true, write: `\r\n${HERO_AGY_BANNER}\r\n\r\n> `, wait: 700 },
  { overlay: null, key: "Enter", wait: 350 },
  { key: null, type: "Build a website for a design studio.", wait: 650 },
  { write: `\r\n${dim("──────────────────────────────────")}\r\n\r\n${blue("⠋")} Building your website…`, wait: 300 },
  { write: `\r\x1b[2K${blue("⠹")} Building your website…`, wait: 300 },
  { write: `\r\x1b[2K${blue("●")} Created index.html ${dim("(+84)")}\r\n`, wait: 200 },
  { write: `${blue("●")} Created styles.css ${dim("(+126)")}\r\n`, wait: 200 },
  { write: `${blue("●")} Started preview server\r\n\r\nReady at ${blue("http://localhost:3000")}\r\n`, wait: 350 },
  { cursor: "link", wait: 650 },
  { click: true, wait: 180 },
  { preview: true, click: false, cursor: "preview", wait: 2500 },
  { preview: false, cursor: "body", wait: 300 },
  { chapter: 2, insert: "work", wait: 1700 },
  { insert: null, overlay: "sessions", key: "⌥ L", caption: "Create another session in the TUI.", wait: 1000 },
  { overlay: "create", key: "⌥ N", profile: "code", input: true, type: HERO_DEMO.tui.email, wait: 1300 },
  { profile: "co-work", key: "↓", wait: 800 },
  { key: "Enter", caption: "Confirm creation → boot a separate VM.", wait: 500 },
  { overlay: null, email: true, workflow: "email", clear: true, write: `${dim("Creating session: Email productivity")}\r\n${dim("Booting a new VM…")}\r\n`, wait: 1200 },
  { overlay: "sessions", key: null, caption: "Email productivity is running. Website stays open.", wait: 1400 },
  { overlay: null, key: "Enter", wait: 300 },
  { key: null, caption: "Start Codex in the new session.", write: prompt, type: "codex", wait: 500 },
  { codex: true, caption: null, clear: true, write: `\x1b[?25l${dim(HERO_DEMO.codex.welcome)}`, wait: 700 },
  { input: true, type: HERO_DEMO.codex.task, wait: 450 },
  { clear: true, write: `› ${HERO_DEMO.codex.task}\r\n\r\n${dim("• Reading your inbox…")}`, wait: 400 },
  { write: `\r\x1b[2K${dim("• Called mail.list_messages")}\r\n${dim("  └ 3 messages returned")}\r\n\r\n`, wait: 200 },
  { write: `${bold("Your inbox, summarized:")}\r\n\r\n`, wait: 100 },
  { write: `• ${bold("Maya")} — website ready for review.\r\n  ${dim("Send feedback today.")}\r\n`, wait: 250 },
  { write: `• ${bold("Design team")} — new concepts ready.\r\n  ${dim("Pick a direction by Friday.")}\r\n`, wait: 250 },
  { write: `• ${bold("Alex")} — coffee on Thursday?\r\n  ${dim("Confirm a time.")}\r\n`, wait: 700 },
  { chapter: 3, insert: "scale", wait: 1700 },
  { insert: null, cursor: "app", wait: 650 },
  { dock: "app", click: true, wait: 220 },
  { surface: "gui", guiView: "sessions", dock: null, click: false, cursor: "body", scale: 2, wait: 700 },
  ...[0, 1].flatMap((index): import("./hero-terminal").TerminalStep[] => [
    { cursor: `tab-${index}`, wait: 300 }, { click: true, wait: 80 },
    { click: false, cursor: "body", guiView: "terminal", guiSession: index, guiStage: "agent", input: true, type: HERO_DEMO.scale.sessions[index].prompt, wait: 150 },
    { launch: index, caption: `${HERO_DEMO.scale.sessions[index].name} · prompt running`, wait: 300 },
  ]),
  ...[2, 3, 4, 5].flatMap((index): import("./hero-terminal").TerminalStep[] => [
    { cursor: "gui-sessions", wait: 250 }, { click: true, wait: 80 },
    { guiView: "sessions", click: false, caption: `Create a new VM for ${HERO_DEMO.scale.sessions[index].name}.`, wait: 150 },
    { cursor: "gui-customize", wait: 250 }, { click: true, wait: 80 },
    { guiView: "create", guiSession: index, click: false, cursor: "gui-name", wait: 300 },
    { click: true, wait: 80 }, { click: false, input: true, type: HERO_DEMO.scale.sessions[index].name, wait: 180 },
    { cursor: "gui-create", wait: 250 }, { click: true, wait: 100 },
    { guiView: "boot", scale: index + 1, click: false, cursor: "body", caption: `New session: ${HERO_DEMO.scale.sessions[index].name} · booting its own VM`, wait: 550 },
    { guiView: "terminal", guiStage: "shell", input: true, type: "codex", wait: 100 },
    { guiStage: "agent", input: true, type: HERO_DEMO.scale.sessions[index].prompt, wait: 100 },
    { launch: index, caption: `${index + 1} prompts running in parallel.`, wait: 300 },
  ]),
  ...[0, 2, 5].flatMap((index): import("./hero-terminal").TerminalStep[] => [
    { cursor: `tab-${index}`, wait: 240 }, { click: true, wait: 100 },
    { click: false, cursor: "body", guiView: "terminal", guiSession: index, guiStage: "agent", caption: HERO_DEMO.scale.parallelCaption, wait: 550 },
  ]),
  { cursor: "gui-sessions", wait: 400 }, { click: true, wait: 100 },
  { click: false, cursor: "body", guiView: "sessions", caption: HERO_DEMO.scale.parallelCaption, wait: 2000 },
];
