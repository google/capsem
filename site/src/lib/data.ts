// All marketing site content lives here. Edit this file to change copy.

export const SITE = {
  name: "Capsem",
  tagline: "The fastest way to ship with AI securely.",
  description:
    "A Rust-powered hypervisor that sandboxes every AI coding agent in its own air-gapped Linux VM. See everything, control everything.",
  installCmd: "curl -fsSL https://capsem.org/install.sh | sh",
  github: "https://github.com/google/capsem",
  docs: "https://docs.capsem.org",
  releases: "https://github.com/google/capsem/releases/latest",
  issues: "https://github.com/google/capsem/issues",
  copyright: "Elie Bursztein",
  license: "MIT",
  platform: "macOS 14+ on Apple Silicon",
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
    title: "Enterprise-grade policy with user and corp config layers",
    description:
      "User-level config in ~/.capsem/user.toml lets developers customize domain lists and HTTP rules. Corp-level config at /etc/capsem/corp.toml (MDM-distributed) locks down policy with enterprise overrides that users cannot bypass.",
    bullets: [
      "Domain allow/block with wildcard support",
      "HTTP method + path matching per domain",
      "Corp config overrides user config entirely",
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
  { port: ":5002", label: "HTTPS" },
  { port: ":5003", label: "MCP" },
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
      "Yes. Edit ~/.capsem/user.toml to define domain allow/block lists and per-domain HTTP rules (method + path matching). For enterprise deployments, /etc/capsem/corp.toml provides lockdown that individual users cannot override.",
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
      { label: "Releases", href: SITE.releases },
    ],
  },
] as const;
