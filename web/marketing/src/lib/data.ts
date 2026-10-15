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
    { id: 'engine', label: `${SECURITY.engine}\n\n${SECURITY.controls.join('\n')}`, x: 415, y: 215, width: 290, height: 180, permission: 'accent' },
    { id: 'credentials', label: `${SECURITY.credentials}\n${SECURITY.outside}`, x: 435, y: 30, width: 250, permission: 'secondary' },
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
  descriptionForScreenReaders: "Animated demonstration: install Capsem, launch the agy image, build and preview a website, then launch a separate Codex VM and summarize fictional emails.",
  pause: "Pause animation", play: "Play animation", replay: "Replay animation",
  chaptersLabel: "Demo chapters",
  chapters: ["Easy to install", "Build fast", "Ship quickly", "Switch agent seamlessly", "Get things done"],
  inserts: {
    build: { eyebrow: "FROM IDEA TO FIRST DRAFT", title: "Build fast.", detail: "Your favorite agent and tools are all ready to go." },
    work: { eyebrow: "NEW TASK — NEW AGENT", title: "Switch workflow instantly", detail: "Capsem offers images for all the agents, tools and workflow you need. Switching is one command away." },
  },
  codex: {
    name: "OpenAI Codex", modelLabel: "model:", model: "gpt-6.1-sol medium",
    directoryLabel: "directory:", directory: "~/inbox", changeModel: "/model to change",
    placeholder: "Ask Codex to do anything", context: "100% context left · ? for shortcuts",
    task: "Summarize my emails.",
    welcome: "To get started, describe a task.\r\n\r\n/model        Choose a model\r\n/permissions  Set permissions",
  },
  shellTitle: "elie — zsh", agyTitle: "website — agy", codexTitle: "inbox — codex",
  previewLabel: "Website preview", previewURL: "localhost:3000",
  preview: { brand: "FORM & FIELD", nav: "Independent design studio", eyebrow: "THOUGHTFUL BY DESIGN", title: "A little space.\nA bigger idea.", description: "We create places, objects, and experiences that make everyday life a little more extraordinary.", action: "Explore our work ↗", footer: "Architecture / Objects / Digital", monogram: "F/f" },
  summary: "Your inbox, summarized:\r\n\r\n• Maya: the website is ready for review.\r\n  Action: send feedback today.\r\n\r\n• Design team: new concepts are ready.\r\n  Action: pick a direction by Friday.\r\n\r\n• Alex: coffee on Thursday?\r\n  Action: confirm a time.\r\n",
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
  { chapter: 0, write: prompt, type: SITE.installCmd, wait: 450 },
  { write: `\r\n${dim("Downloading Capsem")}\r\n`, wait: 350 },
  { write: `${blue("  ███████░░░░░░░░░░░░░  32%")}`, wait: 350 },
  { write: `\r\x1b[2K${blue("  █████████████░░░░░░░  64%")}`, wait: 350 },
  { write: `\r\x1b[2K${blue("  ████████████████████ 100%")}\r\n`, wait: 300 },
  { write: `${dim("Installing CLI…")}\r\n${blue("✓ Capsem installed")}\r\n`, wait: 650 },
  { chapter: 1, insert: "build", wait: 3000 },
  { insert: null, write: prompt, type: "capsem run --image agy", wait: 450 },
  { write: `\r\n${dim("Pulling agy…")}\r\n${dim("Booting VM…")}\r\n`, wait: 850 },
  { title: HERO_DEMO.agyTitle, clear: true, write: `\r\n${agyLogo}\r\n\r\n${bold("Antigravity CLI")}\r\n${dim("Gemini · ~/website")}\r\n\r\n${dim("──────────────────────────────────")}\r\n> `, type: "Build a website for a design studio.", wait: 650 },
  { write: `\r\n${dim("──────────────────────────────────")}\r\n\r\n${blue("⠋")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("⠹")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("●")} Created index.html ${dim("(+84)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Created styles.css ${dim("(+126)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Started preview server\r\n\r\nReady at ${blue("http://localhost:3000")}\r\n`, wait: 1000 },
  { chapter: 2, preview: true, wait: 2500 },
  { preview: false, write: "\r\n> ", type: "/exit", wait: 400 },
  { title: HERO_DEMO.shellTitle, clear: true, write: `\r\n${dim("Workload exited. VM stopped.")}\r\n`, wait: 400 },
  { chapter: 3, insert: "work", wait: 3000 },
  { insert: null, write: prompt, type: "capsem run --image codex", wait: 500 },
  { write: `\r\n${dim("Pulling codex…")}\r\n${dim("Booting VM…")}\r\n`, wait: 850 },
  { chapter: 4, codex: true, title: HERO_DEMO.codexTitle, clear: true, write: `\x1b[?25l${dim(HERO_DEMO.codex.welcome)}`, wait: 800 },
  { input: true, type: HERO_DEMO.codex.task, wait: 450 },
  { clear: true, write: `› ${HERO_DEMO.codex.task}\r\n\r\n${dim("• Reading your inbox…")}`, wait: 750 },
  { write: `\r\x1b[2K${dim("• Called mail.list_messages")}\r\n${dim("  └ 3 messages returned")}\r\n\r\n`, wait: 450 },
  { write: `${bold("Your inbox, summarized:")}\r\n\r\n`, wait: 300 },
  { write: `• ${bold("Maya")} — website ready for review.\r\n  ${dim("Send feedback today.")}\r\n`, wait: 550 },
  { write: `• ${bold("Design team")} — new concepts ready.\r\n  ${dim("Pick a direction by Friday.")}\r\n`, wait: 550 },
  { write: `• ${bold("Alex")} — coffee on Thursday?\r\n  ${dim("Confirm a time.")}\r\n`, wait: 700 },
];
