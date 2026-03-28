# Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework. Built with Rust, Tauri 2.0, and Astro.

## Quick Start

```bash
just doctor        # Check tools (first time)
just build-assets  # Build kernel + rootfs (first time, needs docker/podman)
just run           # Build + boot VM (~10s)
just test          # Unit tests + cross-compile + frontend check
```

See `/dev-just` for the full recipe reference and dependency chains.

## Project Layout

```
crates/capsem-core/       VM library (config, boot, serial, vsock, machine)
crates/capsem-app/        Tauri binary (GUI, CLI, commands, state)
crates/capsem-agent/      Guest PTY agent (vsock bridge, cross-compiled)
frontend/                 Astro 5 + Svelte 5 + Tailwind v4 + Preline
site/                     Product website (Astro Starlight)
src/capsem/builder/       capsem-builder CLI (config-driven image builder)
guest/config/             Guest image configuration (TOML configs)
guest/artifacts/          Guest scripts and diagnostics (capsem-init, bashrc, tests)
assets/                   Built VM assets (gitignored, per-arch: assets/{arch}/)
skills/                   Shared AI agent skills (SKILL.md format)
```

## Skills

Skills live in `skills/` at the project root. Both Claude Code and Gemini CLI discover them via symlinks:

```
skills/<name>/SKILL.md        One skill per directory
.claude/skills -> ../skills   Claude Code symlink
.agents/skills -> ../skills   Gemini CLI symlink
```

Prefix-based grouping: `dev-*`, `build-*`, `release-*`, `site-*`, `frontend-*`, `meta-*`. `asset-pipeline` covers the build-to-boot asset flow. See `/meta-organize-skills` for conventions.

**Do not** put files in `.claude/skills/` or `.agents/skills/` directly -- those are symlinks.

## Skills -- LOAD BEFORE CODING

Skills contain hard-won lessons and project-specific patterns. **Before writing or modifying code, load the relevant skill.** Skipping skills leads to repeated bugs (e.g., blocking async, serde_json::Value on hot paths, missing VM tests).

| Area | Skill | When to load |
|------|-------|--------------|
| Overview | `/dev-capsem` | Orienting on any task, finding which skill to use |
| Rust patterns | `/dev-rust-patterns` | Writing any Rust code in capsem-core/app/agent |
| MITM proxy | `/dev-mitm-proxy` | TLS, HTTP inspection, SSE parsing, ai_traffic |
| MCP gateway | `/dev-mcp` | MCP tool routing, policy, built-in tools |
| Testing | `/dev-testing` | Running or writing tests, TDD, coverage |
| VM testing | `/dev-testing-vm` | In-VM diagnostics, capsem-doctor, session DB |
| Frontend | `/frontend-design` | UI components, Svelte, Tailwind, Preline |
| Build images | `/build-images` | capsem-builder, guest config, rootfs, kernel |
| Initrd repack | `/build-initrd` | Guest binary changes, fast iteration loop |
| Just recipes | `/dev-just` | Which just command to run for a given task |
| Debugging | `/dev-debugging` | Bug investigation, reproduce-first workflow |
| Release | `/release-process` | CI, signing, notarization, changelog |
| Architecture | `/site-architecture` | System design, Tauri, vsock, key files |

## Code Style

- **Reuse over reinvention.** Check `capsem-core` first. Extend existing abstractions.
- **Minimize code.** Delete dead code, inline single-use helpers. Every line must earn its place.
- **`capsem-core` is the shared library.** App crate is a thin Tauri shell. Agent crate is a thin guest binary. Business logic lives in core.
- **One way to do things.** Don't introduce a second pattern when one exists.

## Invariants (do not break)

### Ephemeral VM model

**VirtioFS mode** (default): fresh workspace + sparse rootfs.img per session. Never make the overlay upper persistent.

**Block mode** (legacy): `mke2fs` unconditional at boot. Overlay upper is always tmpfs.

### Guest binary security

All guest binaries deployed chmod 555 (read-only). Rootfs mounted read-only. Guest cannot modify its own binaries.

### Codesigning

The binary must be codesigned with `com.apple.security.virtualization` or VZ calls crash. The justfile handles this.

## Commits

1. Include `CHANGELOG.md` update in the same commit
2. Stage files explicitly (no `git add -A`)
3. Conventional messages: `feat:`, `fix:`, `chore:`, `docs:`
4. Author: Elie Bursztein <github@elie.net>
5. No `Co-Authored-By` trailers

## Logging

Boot sequence instrumented with `tracing` spans. `RUST_LOG=capsem=debug` for full timing, `RUST_LOG=capsem=info` for top-level.
