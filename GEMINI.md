# Gemini CLI Directives for Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework. Built with Rust, Tauri 2.0, and Astro.

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

Skills live in `skills/` (symlinked to `.agents/skills/`). Start with `/dev-capsem` to orient, then load the specific skill for your area.