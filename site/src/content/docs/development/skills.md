---
title: AI Agent Skills
description: How Capsem organizes shared AI coding agent skills for Claude Code and Gemini CLI.
sidebar:
  order: 20
---

Capsem uses a shared `skills/` directory that both Claude Code and Gemini CLI discover via symlinks. One set of files, two consumers, zero duplication.

## Directory structure

```
skills/
  <skill-name>/
    SKILL.md                     The skill (required)
    references/                  Large docs loaded on demand (optional)
    scripts/                     Executable helpers (optional)

.claude/skills -> ../skills      Claude Code symlink
.agents/skills -> ../skills      Gemini CLI symlink
```

Skills are flat (one level). Nested directories are **not** discovered. Use prefix-based naming for categories.

## SKILL.md format

```yaml
---
name: skill-name
description: When to trigger and what it does.
---

# Skill Title

Instructions the agent follows when triggered.
```

The `description` field is the trigger mechanism. Claude sees it in the skill list and decides whether to load the full body. Be specific and slightly pushy -- Claude undertriggers by default.

## Naming conventions

Prefix-based grouping:

| Prefix | Category |
|--------|----------|
| `meta-*` | Skills about skills (find, create, organize) |
| `dev-*` | Development (toolchain, testing, debugging, patterns) |
| `build-*` | VM image building |
| `release-*` | Release process, CI, docs |
| `site-*` | Architecture, documentation site |
| `frontend-*` | Frontend design system |

## Current skills

### Meta
- `meta-find-skills` -- discover community skills via `npx skills`
- `meta-organize-skills` -- skill directory conventions
- `meta-skill-creation` -- create and iterate on skills

### Development
- `dev-capsem` -- project overview and skill navigation map
- `dev-just` -- just recipe reference and dependency chains
- `dev-testing` -- testing policy (TDD, adversarial, 3 tiers)
- `dev-testing-vm` -- capsem-doctor, session inspection, test fixtures
- `dev-testing-hypervisor` -- KVM, Apple VZ, VirtioFS testing
- `dev-testing-frontend` -- vitest, visual verification
- `dev-debugging` -- reproduce, diagnose, fix methodology
- `dev-capsem-doctor` -- in-VM diagnostic suite reference
- `dev-session-debug` -- session DB schema, telemetry debugging
- `dev-setup` -- new developer onboarding
- `dev-sprint` -- sprint planning and workflow
- `dev-rust-patterns` -- async/tokio, cross-compile, error handling
- `dev-mitm-proxy` -- MITM proxy pipeline, SSE parsing, provider wire formats
- `dev-mcp` -- MCP gateway, JSON-RPC, tool routing
- `dev-skills` -- how skills work (for building Capsem's own skills system)

### Build
- `build-images` -- capsem-builder CLI, guest config
- `build-initrd` -- guest binary repack, fast iteration

### Release
- `release-process` -- release, CI, Apple signing, docs, changelog

### Site
- `site-architecture` -- system architecture, key files, Tauri reference
- `site-infra` -- Astro Starlight docs site conventions

### Frontend
- `frontend-design` -- design system, Preline, color scheme, Svelte 5 rune patterns

## Progressive disclosure

Skills load in three tiers:

1. **Metadata** (~100 words) -- name + description, always in context
2. **SKILL.md body** (<500 lines) -- loaded when skill triggers
3. **Bundled resources** (unlimited) -- `references/` files, loaded on demand

Keep SKILL.md lean. Put wire formats, API docs, and community references in `references/`.

## Adding a skill

```bash
mkdir skills/<prefix-name>
# Write skills/<prefix-name>/SKILL.md with frontmatter
# Available immediately (live reload, no restart)
```

## Community skills

Search with `npx skills find <query>`. Place community skills as references, not top-level:

```bash
curl -sL https://raw.githubusercontent.com/<owner>/<repo>/main/<path>/SKILL.md \
  -o skills/<name>/references/<topic>.md
```

## Global skills

Skills in `~/.claude/skills/` are available across all projects. We install meta skills globally:
- `meta-find-skills`
- `meta-organize-skills`
- `meta-skill-creation`
