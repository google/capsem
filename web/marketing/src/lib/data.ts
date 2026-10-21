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
