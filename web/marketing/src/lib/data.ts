// All marketing site content lives here. Edit this file to change copy.

export const SITE = {
  name: "Capsem",
  tagline: "The fastest way to ship with AI securely.",
  description:
    "A Rust-powered hypervisor that sandboxes every AI coding agent in its own air-gapped Linux VM. See everything, control everything.",
  installCmd: "curl -fsSL https://capsem.org/install.sh | sh",
  github: "https://github.com/google/capsem",
  docs: "https://docs.capsem.org",
  releases: "https://release.capsem.org/channels/stable/",
  issues: "https://github.com/google/capsem/issues",
  copyright: "Elie Bursztein",
  license: "MIT",
  platform: "macOS 14+ on Apple Silicon or Debian/Ubuntu Linux with KVM",
} as const;

export const NAV_LINKS = [
  { label: "Features", href: "#features" },
  { label: "How It Works", href: "#how-it-works" },
  { label: "FAQ", href: "#faq" },
  { label: "Docs", href: SITE.docs },
] as const;

export const AGENTS = [
  { name: "Claude Code", provider: "Anthropic" },
  { name: "Gemini CLI", provider: "Google" },
  { name: "Codex", provider: "OpenAI" },
] as const;

export const MCP_TOOLS = [
  { name: "fetch_http", desc: "Fetch and extract web content" },
  { name: "grep_http", desc: "Search web pages with regex" },
  { name: "http_headers", desc: "Inspect HTTP headers and status" },
] as const;

export const PACKAGES = [
  "Python 3", "Node.js 24", "git", "uv",
  "numpy", "pandas", "scipy", "scikit-learn",
  "requests", "httpx", "beautifulsoup4",
  "pytest", "rich", "matplotlib", "fastmcp",
] as const;

export const ROADMAP = [
  "VM checkpointing and restore",
  "Linux host support",
  "VS Code extension",
  "Custom MCP server marketplace",
] as const;

export const SECURITY_BLOCKS = [
  {
    badge: "ISOLATION",
    title: "Hardware-level sandboxing with Apple Virtualization.framework",
    description:
      "Each agent session boots a lightweight Linux VM with a read-only rootfs, no swap, no kernel modules, no debugfs. Air-gapped networking with a dummy NIC and fake DNS ensures nothing reaches the real network without going through the MITM proxy.",
    bullets: [
      "Ephemeral VMs -- fresh state every session",
      "Read-only rootfs with tmpfs workspace",
      "No systemd, no sshd, no cron -- minimal attack surface",
    ],
  },
  {
    badge: "INSPECTION",
    title: "See everything your AI agent does on the network",
    description:
      "A transparent MITM proxy terminates TLS from the guest using per-domain minted certificates, inspects every HTTP request and response, and applies policy before forwarding to the real upstream. Full request/response bodies are logged to a per-session SQLite database.",
    bullets: [
      "Per-domain TLS certificate minting",
      "Method + path policy rules per domain",
      "Full body capture for post-hoc analysis",
    ],
  },
  {
    badge: "CONTROL",
    title: "Enterprise-grade policy with profile and corp config layers",
    description:
      "Each profile owns its enforcement, detection, MCP, plugin, and asset configuration. Corp-level config at /etc/capsem/corp.toml (MDM-distributed) locks down policy and reporting with enterprise overrides that users cannot bypass.",
    bullets: [
      "Profile-owned CEL enforcement rules",
      "Detection rules with Sigma import/export",
      "Corp config locks and reports profile behavior",
    ],
  },
] as const;

export const HOST_COMPONENTS = [
  { label: "Desktop App", detail: "GUI + CLI interface", icon: "monitor" },
  { label: "MITM Proxy", detail: "TLS termination + HTTP inspection", icon: "shield" },
  { label: "Policy Engine", detail: "Domain + HTTP + MCP rules", icon: "file-text" },
  { label: "Session Telemetry", detail: "SQLite DB per session", icon: "bar-chart" },
] as const;

export const GUEST_COMPONENTS = [
  { label: "AI Agent", detail: "Claude / Gemini / Codex", icon: "terminal" },
  { label: "PTY Agent", detail: "Terminal I/O over vsock", icon: "grid" },
  { label: "Net Proxy", detail: "TCP-to-vsock relay (iptables)", icon: "layers" },
  { label: "MCP Server", detail: "Tool relay over vsock", icon: "settings" },
] as const;

export const VSOCK_CHANNELS = [
  { port: ":5001", label: "terminal" },
  { port: ":5002", label: "HTTPS + MCP" },
] as const;

export const FAQS = [
  {
    question: "Does Capsem work with Claude Code, Gemini CLI, and Codex?",
    answer:
      "Yes. Capsem supports any AI coding agent that runs in a terminal. Claude Code, Gemini CLI, and Codex are pre-installed in the VM and configured to work through the MITM proxy automatically.",
  },
  {
    question: "How does the MITM proxy work?",
    answer:
      "All guest HTTPS traffic is redirected through an iptables rule to a local TCP relay, which bridges to the host via vsock. The host terminates TLS using per-domain minted certificates (signed by a static Capsem CA baked into the guest's trust store), inspects the HTTP request, applies policy, and forwards to the real upstream.",
  },
  {
    question: "What platforms are supported?",
    answer:
      "Capsem requires macOS on Apple Silicon (M1 or later). It uses Apple's Virtualization.framework which is only available on macOS. The guest VM runs aarch64 Linux.",
  },
  {
    question: "Can I customize which domains are allowed?",
    answer:
      "Yes. Edit the profile's enforcement rules to control HTTP, DNS, MCP, model, file, process, IP, and transport behavior. For enterprise deployments, /etc/capsem/corp.toml provides lockdown that individual users cannot override.",
  },
  {
    question: "Is the VM truly air-gapped?",
    answer:
      "Yes. The guest has no real network interface. It uses a dummy NIC with fake DNS (dnsmasq) and iptables rules that redirect all port 443 traffic through the MITM proxy. Direct IP access and non-443 ports are blocked entirely.",
  },
] as const;

export const FOOTER_COLUMNS = [
  {
    title: "Product",
    links: [
      { label: "Features", href: "#features" },
      { label: "How It Works", href: "#how-it-works" },
      { label: "FAQ", href: "#faq" },
    ],
  },
  {
    title: "Resources",
    links: [
      { label: "Documentation", href: SITE.docs },
      { label: "GitHub", href: SITE.github },
      { label: "Issues", href: SITE.issues },
      { label: "Release Channel", href: SITE.releases },
    ],
  },
] as const;


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
