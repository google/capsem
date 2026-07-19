# Gemini CLI Directives for Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework. Built with Rust, Tauri 2.0, and Astro.

Shared agent invariants live in `AGENTS.md`; it is the Codex/Claude/Gemini
common contract. **Read it before any release work or any change touching
logged data** -- it carries the two hard contracts: release evidence (only the
successful remote `release-qualification.yaml` run on the exact candidate SHA
counts; never a local run, a nearby commit's green, or an agent's claim) and
the logger DB boundary (only `capsem-logger` executes ledger queries).

## Skills -- LOAD BEFORE CODING

Skills contain hard-won lessons and project-specific patterns. **Before writing or modifying code, load the relevant skill.** Skipping skills leads to repeated bugs (e.g., blocking async, serde_json::Value on hot paths, missing VM tests).

| Area | Skill | When to load |
|------|-------|--------------|
| Overview | `/dev-capsem` | Orienting on any task, finding which skill to use |
| Quick start | `/dev-start` | First-time bootstrap, onboarding |
| Dev setup | `/dev-setup` | Environment setup, tool install, troubleshooting |
| Rust patterns | `/dev-rust-patterns` | Writing any Rust code in capsem-core/app/agent |
| MITM proxy | `/dev-mitm-proxy` | TLS, HTTP inspection, SSE parsing, ai_traffic |
| MCP | `/dev-mcp` | capsem-mcp server, MCP gateway, aggregator, builtin, tool routing |
| Testing | `/dev-testing` | Running or writing tests, TDD, coverage |
| VM testing | `/dev-testing-vm` | In-VM diagnostics, capsem-doctor, session DB |
| Hypervisor testing | `/dev-testing-hypervisor` | Apple VZ / KVM, VirtioFS, vsock tests |
| Frontend testing | `/dev-testing-frontend` | vitest, svelte-check, visual verification |
| Python testing | `/dev-testing-python` | capsem-builder pytest, coverage, golden fixtures |
| Session DB | `/dev-session-debug` | Inspecting session.db, correlating events |
| Benchmarking | `/dev-benchmark` | capsem-bench, performance regression |
| capsem-doctor | `/dev-capsem-doctor` | In-VM diagnostic suite, adding new tests |
| Frontend | `/frontend-design` | UI components, Svelte 5 runes, Tailwind, Preline |
| Build images | `/build-images` | capsem-builder, guest config, rootfs, kernel |
| Initrd repack | `/build-initrd` | Guest binary changes, fast iteration loop |
| Asset pipeline | `/asset-pipeline` | Asset manifest, hash verification, boot-time resolution |
| Just recipes | `/dev-just` | Which just command to run for a given task |
| Debugging | `/dev-debugging` | Bug investigation, reproduce-first workflow |
| CI triage | `/dev-ci` | Red gates, pr-gate failures, rerun decisions, stop-the-line policy |
| Sprints | `/dev-sprint` | Running a multi-step feature sprint |
| Release | `/release-process` | CI, signing, notarization, changelog |
| Release gate proof | `/ironbank` | Black-box acceptance proof for VM, network, MCP, security, or release-gate behavior |
| Bug queue | `/dev-bug-review` | Working a queue of bug reports one-by-one (confirm, push back, fix, commit) |
| Installation | `/dev-installation` | Setup wizard, service registration, self-update, install tests |
| Architecture | `/site-architecture` | System design, service architecture, vsock, key files |
| Docs site | `/site-infra` | Writing/editing docs, Starlight, sidebar, release pages |
| Marketing site | `/site-marketing` | Marketing website (capsem.org), copy, components, theme |
| Skills system | `/dev-skills` | How skills work, naming, discovery |
| Skills layout | `/meta-organize-skills` | Skills directory conventions, symlinks |
| Skill discovery | `/meta-find-skills` | Finding or installing skills from the ecosystem |
| Skill authoring | `/meta-skill-creation` | Creating, improving, or evaluating skills |

Skills live in repository `skills/`. Start with `/dev-capsem` to orient, then load the specific skill for your area. Do not mirror developer skills under `config/skills`.
