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
  descriptionForScreenReaders: "Four chapters: install Capsem, ship a website with AGY, swap to a Codex image to summarize fictional emails, then run six agents in parallel in separate VMs. Manage them with Capsem's GUI, TUI, API, or ask your agent through MCP.",
  pause: "Pause animation", play: "Play animation", replay: "Replay animation",
  chaptersLabel: "Demo chapters",
  chapters: ["Install", "Ship fast", "Swap workflow", "Scale"],
  desktop: { terminal: "Terminal", browser: "Browser", app: "Capsem", menus: ["File", "Edit", "View"], clock: "Mon 9:41", },
  inserts: {
    build: { eyebrow: "FROM IDEA TO WEBSITE", title: "Ship fast.", detail: "Your favorite agent and tools are all ready to go." },
    work: { eyebrow: "NEW TASK — NEW AGENT", title: "Swap workflow.", detail: "Your next agent, tools and connections. One command away." },
  },
  scale: {
    label: "Capsem managing parallel agent sessions", controlsLabel: "Control Capsem your way",
    detail: "More agents. Separate VMs. All in one place.",
    tab: "Sessions", new: "+ New", running: "Running", booting: "Booting",
    columns: ["Name", "Status", "Image", "Uptime", "Actions"],
    active: "agents running", isolated: "isolated VMs", queued: "Starting agents…",
    surfaces: [{ id: "gui", label: "GUI" }, { id: "tui", label: "TUI" }, { id: "api", label: "API" }, { id: "mcp", label: "Ask your agent · MCP" }],
    tui: { title: "TUI — capsem shell", heading: "sessions", shortcut: "Alt+L sessions   Alt+N new   Alt+←/→ switch", columns: ["Name", "Image", "State", "Time"] },
    api: { title: "API — @capsem/sdk", code: 'import { Hypervisor } from "@capsem/sdk";\n\nconst hv = new Hypervisor(url, token);\nconst { sandboxes } = await hv.list();\n\nconsole.log(sandboxes.length);', result: "6 sessions · all running", endpoint: "GET /vms/list  200 OK" },
    mcp: { title: "Ask your agent — MCP", prompt: "Check on all my agents.", call: "Called capsem.capsem_list({})", result: "All 6 agents are running in separate VMs.", detail: "Website, inbox, API, tests, docs and review.\nEverything is moving in parallel." },
    sessions: [
      { name: "Website", image: "agy", agent: "Antigravity", task: "Building the homepage", uptime: "00:24" },
      { name: "Inbox", image: "codex", agent: "Codex", task: "Summarizing your emails", uptime: "00:21" },
      { name: "API", image: "claude", agent: "Claude Code", task: "Implementing the API", uptime: "00:18" },
      { name: "Tests", image: "codex", agent: "Codex", task: "Running the test suite", uptime: "00:16" },
      { name: "Docs", image: "gemini", agent: "Gemini CLI", task: "Updating documentation", uptime: "00:12" },
      { name: "Review", image: "claude", agent: "Claude Code", task: "Reviewing the changes", uptime: "00:08" },
    ],
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
  { chapter: 1, insert: "build", wait: 1800 },
  { insert: null, write: prompt, type: "capsem run --image agy", wait: 450 },
  { write: `\r\n${dim("Pulling agy…")}\r\n${dim("Booting VM…")}\r\n`, wait: 850 },
  { title: HERO_DEMO.agyTitle, clear: true, write: `\r\n${agyLogo}\r\n\r\n${bold("Antigravity CLI")}\r\n${dim("Gemini · ~/website")}\r\n\r\n${dim("──────────────────────────────────")}\r\n> `, type: "Build a website for a design studio.", wait: 650 },
  { write: `\r\n${dim("──────────────────────────────────")}\r\n\r\n${blue("⠋")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("⠹")} Building your website…`, wait: 650 },
  { write: `\r\x1b[2K${blue("●")} Created index.html ${dim("(+84)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Created styles.css ${dim("(+126)")}\r\n`, wait: 450 },
  { write: `${blue("●")} Started preview server\r\n\r\nReady at ${blue("http://localhost:3000")}\r\n`, wait: 1000 },
  { preview: true, wait: 2500 },
  { preview: false, write: "\r\n> ", type: "/exit", wait: 400 },
  { title: HERO_DEMO.shellTitle, clear: true, write: `\r\n${dim("Workload exited. VM stopped.")}\r\n`, wait: 400 },
  { chapter: 2, insert: "work", wait: 1800 },
  { insert: null, write: prompt, type: "capsem run --image codex", wait: 500 },
  { write: `\r\n${dim("Pulling codex…")}\r\n${dim("Booting VM…")}\r\n`, wait: 850 },
  { codex: true, title: HERO_DEMO.codexTitle, clear: true, write: `\x1b[?25l${dim(HERO_DEMO.codex.welcome)}`, wait: 800 },
  { input: true, type: HERO_DEMO.codex.task, wait: 450 },
  { clear: true, write: `› ${HERO_DEMO.codex.task}\r\n\r\n${dim("• Reading your inbox…")}`, wait: 750 },
  { write: `\r\x1b[2K${dim("• Called mail.list_messages")}\r\n${dim("  └ 3 messages returned")}\r\n\r\n`, wait: 450 },
  { write: `${bold("Your inbox, summarized:")}\r\n\r\n`, wait: 300 },
  { write: `• ${bold("Maya")} — website ready for review.\r\n  ${dim("Send feedback today.")}\r\n`, wait: 550 },
  { write: `• ${bold("Design team")} — new concepts ready.\r\n  ${dim("Pick a direction by Friday.")}\r\n`, wait: 550 },
  { write: `• ${bold("Alex")} — coffee on Thursday?\r\n  ${dim("Confirm a time.")}\r\n`, wait: 700 },
  { chapter: 3, codex: false, preview: false, insert: null, surface: "gui", scale: 0, wait: 450 },
  { scale: 2, wait: 650 },
  { scale: 4, wait: 650 },
  { scale: HERO_DEMO.scale.sessions.length, wait: 2200 },
  { surface: "tui", wait: 2500 },
  { surface: "api", wait: 2800 },
  { surface: "mcp", wait: 4000 },
];
