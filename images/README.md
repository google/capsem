# Capsem VM Images

This directory contains everything that goes into the guest Linux VM: the kernel config, rootfs Dockerfile, init script, shell config, and package manifests.

## Guest Environment

The VM runs Debian bookworm-slim on aarch64 with a **read-only rootfs**. Only `/root` (scratch disk or tmpfs), `/tmp`, `/run`, and `/var` are writable.

### Package Installation

| Method | Works? | Details |
|--------|--------|---------|
| `pip install <pkg>` | Yes | Installs to `/root/.venv` (auto-activated at boot) |
| `uv pip install <pkg>` | Yes | Same venv, faster |
| `npm install <pkg>` | Yes | Local install to `./node_modules/` |
| `npm install -g <pkg>` | Yes | Installs to `/root/.npm-global/` (writable scratch disk) |
| `apt install <pkg>` | No | Rootfs is read-only. Add packages to `Dockerfile.rootfs` and rebuild with `just build`. |

### Pre-installed Packages

Python packages and npm globals are declared in manifest files so they're easy to version and edit:

- **`requirements.txt`** -- Python packages installed system-wide in the rootfs. The boot-time venv inherits them via `--system-site-packages`. Agents can install additional packages at runtime with `pip` or `uv`.
- **`npm-globals.txt`** -- npm packages (AI CLIs) installed to `/opt/ai-clis/` during Docker build. Copied to the writable scratch disk at boot so they can self-update.

### Python Environment

A virtualenv is created at `/root/.venv` during boot (using `uv venv`, ~100ms) with `--system-site-packages` so pre-installed packages from the rootfs are available immediately. The venv is activated in `capsem-bashrc`. New `pip install` / `uv pip install` commands write to the venv on the scratch disk.

### npm Environment

The npm global prefix is set to `/root/.npm-global/` at boot. AI CLIs (claude, gemini, codex) are pre-seeded there from the rootfs staging area (`/opt/ai-clis/`). This allows them to self-update at runtime since the scratch disk is writable. Runtime `npm install -g` also goes here.

### AI CLI Status

At boot, the host injects environment variables indicating which AI providers are enabled:

| Env Var | Source |
|---------|--------|
| `CAPSEM_ANTHROPIC_ALLOWED` | `ai.anthropic.allow` setting (1 or 0) |
| `CAPSEM_OPENAI_ALLOWED` | `ai.openai.allow` setting (1 or 0) |
| `CAPSEM_GOOGLE_ALLOWED` | `ai.google.allow` setting (1 or 0) |
| `ANTHROPIC_API_KEY` | `ai.anthropic.api_key` setting |
| `OPENAI_API_KEY` | `ai.openai.api_key` setting |
| `GEMINI_API_KEY` | `ai.google.api_key` setting |

The login banner (`capsem-bashrc`) shows each tool's status:
- **ready** (blue) -- provider allowed and API key configured
- **no api key -- configure in settings** (purple) -- provider allowed but no key
- **disabled by policy** (purple) -- provider blocked in user/corp settings

### Login Banner

The login experience is composed of three files:

- **`banner.txt`** -- Static banner shown at login. Supports `%KERNEL%` and `%ARCH%` placeholders replaced at runtime.
- **`tips.txt`** -- Developer tips, one per line (`#` for comments). A random tip is shown each login.
- **`capsem-bashrc`** -- Shell config that renders the banner, tip, and AI status section.

## Files

| File | Purpose | Rebuild |
|------|---------|---------|
| `Dockerfile.rootfs` | Rootfs image (packages, tools, CLIs) | `just build` |
| `Dockerfile.kernel` | Custom Linux kernel | `just build` |
| `capsem-init` | PID 1 init script (mounts, networking, agent launch) | `just repack` |
| `capsem-bashrc` | Guest shell config (venv, npm prefix, banner, AI status) | `just build` |
| `banner.txt` | Login banner | `just build` |
| `tips.txt` | Random developer tips | `just build` |
| `requirements.txt` | Pre-installed Python packages | `just build` |
| `npm-globals.txt` | Pre-installed npm global packages (AI CLIs) | `just build` |
| `capsem-doctor` | In-VM diagnostic runner | `just build` |
| `diagnostics/` | pytest-based VM test suite | `just build` |
| `build.py` | Build orchestrator (kernel, rootfs, initrd) | -- |
| `defconfig` | Kernel .config | `just build` |
