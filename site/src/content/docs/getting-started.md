---
title: Getting Started
description: Install Capsem and boot your first sandboxed AI agent session.
---

## Requirements

| | macOS | Linux |
|---|---|---|
| **OS** | macOS 14 (Sonoma) or later | Debian/Ubuntu (apt-based) |
| **Hardware** | Apple Silicon (M1 or newer) | x86_64 or arm64, KVM capable |
| **Disk** | ~2 GB for the app + VM assets | ~2 GB for the app + VM assets |

macOS uses Apple's Virtualization.framework (Apple Silicon only). Linux uses KVM.

## Install

### One-liner (recommended)

```sh
curl -fsSL https://capsem.org/install.sh | sh
```

The script auto-detects your OS and architecture:
- **macOS**: downloads the `.dmg`, mounts it, and copies `Capsem.app` to `/Applications`
- **Linux**: downloads the `.deb` package and installs it via `apt`

### Manual download

1. Go to the [latest release](https://github.com/google/capsem/releases/latest) on GitHub.
2. Download the `.dmg` (macOS) or `.deb` (Linux) file for your architecture.
3. macOS: open the DMG and drag **Capsem.app** to `/Applications`.
4. Linux: `sudo apt install ./capsem_*.deb`

### Building from source

See the [Development Guide](/development/getting-started/) for instructions on cloning the repo, installing toolchain dependencies, building VM assets, and running from source.

## First session

Launch Capsem from your application menu or the command line:

```sh
# macOS
open /Applications/Capsem.app

# Linux
capsem
```

Or use CLI mode directly:

```sh
# macOS
/Applications/Capsem.app/Contents/MacOS/capsem

# Linux
capsem
```

On first launch, Capsem boots a lightweight Linux VM with an air-gapped network. You get a terminal inside the sandbox with Python 3, Node.js, git, and 30+ packages pre-installed.

## Using an AI agent

Capsem comes with Claude Code, Gemini CLI, and Codex pre-installed in the VM. To start a session with an agent:

```sh
# Inside the Capsem terminal
claude    # Claude Code
gemini    # Gemini CLI
codex     # Codex
```

API keys are configured in `~/.capsem/user.toml` on the host:

```toml
[ai.anthropic]
api_key = "sk-ant-..."

[ai.google]
api_key = "AIza..."

[ai.openai]
api_key = "sk-..."
```

The keys are securely forwarded into the VM at boot time. They never touch the guest filesystem.

## Network policy

By default, the VM is air-gapped -- all network traffic routes through the host's MITM proxy. Only explicitly allowed domains can be reached. Configure allowed domains in `~/.capsem/user.toml`:

```toml
[network]
allowed_domains = [
  "api.anthropic.com",
  "generativelanguage.googleapis.com",
  "api.openai.com",
  "pypi.org",
  "files.pythonhosted.org",
  "registry.npmjs.org",
]
```

Every HTTPS request is logged to a per-session SQLite database with full method, path, headers, and body preview. The Capsem GUI shows this in real time in the Network tab.

## What's next

- [Kernel Hardening](/security/kernel-hardening/) -- how the VM kernel is locked down
- [Network Isolation](/security/network-isolation/) -- air-gapped networking and the MITM proxy
- [Capsem Doctor](/debugging/capsem-doctor/) -- run diagnostics inside the VM
