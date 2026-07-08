---
title: Getting Started
description: Install Capsem and boot your first sandboxed AI agent session.
sidebar:
  order: 0
---

## Requirements

| | macOS | Linux |
|---|---|---|
| **OS** | macOS 14 (Sonoma) or later | Debian/Ubuntu (apt-based) |
| **Hardware** | Apple Silicon (M1 or newer) | x86_64 or arm64, KVM capable |
| **Disk** | ~2 GB for binaries + VM assets | ~2 GB for binaries + VM assets |

macOS uses Apple's Virtualization.framework (Apple Silicon only). Linux uses KVM.

## Install

### One-liner (recommended)

```sh
curl -fsSL https://capsem.org/install.sh | sh
```

The script auto-detects your OS and architecture, installs the Capsem binaries,
and registers the background service. VM assets are downloaded and verified
through the service asset contract.

### Manual download

1. Go to the [stable release channel](https://release.capsem.org/channels/stable/).
2. Download the `.pkg` (macOS) or `.deb` (Linux) file for your architecture.
3. macOS: open the package installer and follow the prompts.
4. Linux: `sudo apt install ./capsem_*.deb`

### Building from source

See the [Development Guide](/development/getting-started/) for instructions on cloning the repo, installing toolchain dependencies, building VM assets, and running from source.

## Service And Assets

After install, the Capsem service runs in the background and starts
automatically on login. The desktop UI and CLI report asset status while the
kernel, initrd, and rootfs download in the background.

```sh
capsem status
capsem start
```

## First session

Boot a sandboxed VM and get a shell:

```sh
capsem shell
```

This creates a Linux session with an air-gapped network. You get a terminal
inside the sandbox with Python 3, Node.js, git, and common developer packages
pre-installed. The default session uses the `code` profile.

For a named retained session that survives stop/resume cycles:

```sh
capsem create -n mybox
capsem shell mybox
```

Manage sessions with:

```sh
capsem list               # show all sessions
capsem info mybox         # detailed status + telemetry
capsem suspend mybox      # save state to disk
capsem resume mybox       # resume from where you left off
capsem delete mybox       # destroy permanently
```

See the [CLI Reference](/usage/cli/) for the full command list.

### Desktop app

You can also use the Capsem desktop app for a graphical interface:

```sh
# macOS
open /Applications/Capsem.app

# Or launch from the system tray
```

The desktop app connects to the same background service -- it's a thin browser shell showing the same UI.

## Using an AI agent

Capsem comes with Claude Code, Gemini CLI, and Codex pre-installed in the VM. To start a session with an agent:

```sh
# Inside the Capsem terminal
claude    # Claude Code
gemini    # Gemini CLI
codex     # Codex
```

API keys can be configured by the tool inside the VM or brokered by Capsem when
observed at a supported boundary. Brokered credentials are stored and logged
only as BLAKE3 references; raw credentials stay broker-private and are not
materialized as settings-owned boot secrets.

## Network policy

By default, the VM is air-gapped -- network traffic routes through Capsem's host
network engine, where HTTP and DNS become first-party security events. Add
allow/block behavior with profile or corp enforcement rules:

```toml
[profiles.rules.allow_python_registry]
name = "allow_python_registry"
action = "allow"
match = 'http.host.matches("^(pypi\\.org|files\\.pythonhosted\\.org)$")'

[profiles.rules.block_unapproved_ai_dns]
name = "block_unapproved_ai_dns"
action = "block"
match = 'dns.qname.matches("(^|.*\\.)(openai\\.com|anthropic\\.com|googleapis\\.com)$")'
```

Every HTTP/DNS/model/MCP/file/process boundary is logged to a per-VM SQLite
database when observed. The Capsem GUI shows this in the VM Stats tab through
typed ledger routes backed by the logger DB API; raw SQL is a local developer
forensic tool, not a Capsem route or UI surface.

## MCP integration

AI agents can control sandboxes programmatically via the MCP server:

```sh
# Add to your Claude Code or Gemini CLI MCP config
capsem-mcp
```

This gives agents tools to create, exec, read/write files, and inspect sessions.

## What's next

- [CLI Reference](/usage/cli/) -- full command reference
- [Service Architecture](/architecture/service-architecture/) -- how the multi-binary system works
- [Kernel Hardening](/security/kernel-hardening/) -- how the VM kernel is locked down
- [Network Isolation](/security/network-isolation/) -- air-gapped networking and the MITM proxy
- [Capsem Doctor](/debugging/capsem-doctor/) -- run diagnostics inside the VM
