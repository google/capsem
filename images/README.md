# Capsem VM Images

This directory contains everything that goes into the guest Linux VM: the kernel config, rootfs Dockerfile, init script, shell config, and package manifests.

## Guest Environment

The VM runs Debian bookworm-slim on aarch64 with a **read-only rootfs**. Only `/root` (scratch disk or tmpfs), `/tmp`, `/run`, and `/var` are writable.

### Adding Packages to the VM

Each package ecosystem has a manifest file. Edit the relevant file and run `just build-assets` to rebuild the rootfs. All installs are baked into the squashfs — they survive reboots but are read-only at runtime (writes go to the overlayfs upper layer on tmpfs).

#### `apt-packages.txt` — System packages (Debian apt)

```
# add a line:
ripgrep
```

Then `just build-assets`. Sources use HTTPS (`deb.debian.org`, `security.debian.org`). Both domains are in the default allow list so they work through the MITM proxy. The package lists are pre-populated at build time so `apt-get install` works inside a running VM without a prior `apt-get update` (lists will be as fresh as the last `just build-assets`).

#### `requirements.txt` — Python packages (pip/uv)

```
# add a line:
httpx
```

Then `just build-assets`. Packages are installed system-wide; the boot-time venv at `/root/.venv` inherits them via `--system-site-packages`. Agents can also install additional packages at runtime with `pip install` or `uv pip install` — those go to the venv on the writable scratch disk.

#### `npm-globals.txt` — npm global packages (AI CLIs)

```
# add a line:
@anthropic-ai/claude-code
```

Then `just build-assets`. Packages are installed to `/opt/ai-clis/` during the Docker build and copied to the writable scratch disk at boot (`/root/.npm-global/`) so they can self-update at runtime.

#### Runtime installs (session-only, gone after shutdown)

| Command | Where it goes | Persists? |
|---------|--------------|-----------|
| `pip install <pkg>` | `/root/.venv` (scratch disk) | No |
| `uv pip install <pkg>` | `/root/.venv` (scratch disk) | No |
| `npm install <pkg>` | `./node_modules/` | No |
| `npm install -g <pkg>` | `/root/.npm-global/` (scratch disk) | No |
| `apt-get install <pkg>` | overlayfs upper (tmpfs) | No |

All runtime installs are ephemeral — the scratch disk and tmpfs upper layer are wiped on every boot.

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
| `Dockerfile.rootfs` | Rootfs image (packages, tools, CLIs) | `just build-assets` |
| `Dockerfile.kernel` | Custom Linux kernel | `just build-assets` |
| `capsem-init` | PID 1 init script (mounts, networking, agent launch) | `just run` |
| `capsem-bashrc` | Guest shell config (venv, npm prefix, banner, AI status) | `just build-assets` |
| `banner.txt` | Login banner | `just build-assets` |
| `tips.txt` | Random developer tips | `just build-assets` |
| `apt-packages.txt` | Pre-installed system packages (apt) | `just build-assets` |
| `requirements.txt` | Pre-installed Python packages | `just build-assets` |
| `npm-globals.txt` | Pre-installed npm global packages (AI CLIs) | `just build-assets` |
| `capsem-doctor` | In-VM diagnostic runner | `just build-assets` |
| `diagnostics/` | pytest-based VM test suite | `just build-assets` |
| `build.py` | Build orchestrator (kernel, rootfs, initrd) | -- |
| `defconfig` | Kernel .config | `just build-assets` |
