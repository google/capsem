---
title: Installation
description: How do I install Capsem on macOS or Linux?
sidebar:
  order: 1
next:
  label: How to use Capsem
  link: /getting-started/how-to-use-capsem/
---

## Requirements

| Operating system | Hardware |
| --- | --- |
| macOS 14 (Sonoma) or later | Apple Silicon (M1 or newer) |
| Ubuntu 24.04 or later | x86_64 or arm64 with KVM |
| Debian 13 or later | x86_64 or arm64 with KVM |

You need an internet connection, `curl`, and permission to run `sudo`.
Linux installation uses `apt` and requires a systemd user session, KVM access,
and Landlock ABI 6 or later with seccomp enabled. A distribution name alone
does not establish that its installed kernel supports worker confinement.
Intel Macs are not supported.

## Install

Run this in a terminal as your regular user:

```sh
curl -fsSL https://capsem.org/install.sh | sh
```

The installer selects the stable release for your operating system and
architecture, verifies the package, and installs it using the native package
manager. It downloads the VM runtime assets and registers the background
service to start automatically when you log in. Enter your password if prompted
by `sudo`, and wait for installation to finish.

You can also download the matching package from the
[stable release channel](https://release.capsem.org/channels/stable/):

- **macOS:** open the `.pkg` and follow the installer prompts.
- **Linux:** download the `.deb` for your architecture, then run
  `sudo apt install ./capsem_*.deb` in the download directory.

## Check the installation

Open a new terminal and run:

```sh
capsem version
capsem status
```

`capsem version` prints the installed version. In `capsem status`, check that
the service reports `ok` and the assets report `ready`. Resolve any reported
errors before creating your first VM.

## Next steps

[How to use Capsem](/getting-started/how-to-use-capsem/) walks you through
launching and keeping your first VM. You can also go directly to:

- [Guides](/guides/) for task-based how-tos.
- [Reference](/reference/) for CLI, MCP, and configuration contracts.
- [SDK](/sdk/) to control Capsem from your application.
