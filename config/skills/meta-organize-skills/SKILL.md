---
name: meta-organize-skills
description: Use when creating, reorganizing, or maintaining the config/skills/ directory. Covers the shared skill layout conventions, directory structure, SKILL.md format, canonical source ownership, and how to add or restructure skills for Capsem agent/profile injection.
---

# Organize Skills

This project uses `config/skills/` as the canonical checked-in skill library.
Agent-specific discovery or guest injection copies or mounts from this path
explicitly. Do not add root dot-dir symlinks as product truth.

## Directory structure

```
config/skills/                   Canonical location (checked into git)
  <skill-name>/
    SKILL.md                     Required -- the skill itself
    references/                  Optional -- large docs loaded on demand
    scripts/                     Optional -- executable helpers
    assets/                      Optional -- templates, icons, etc.
```

Rules:
- One skill per directory. The directory name is the skill identifier.
- Every skill directory must contain a `SKILL.md` file. No other naming is discovered.
- Never put skill source files directly in `.claude/`, `.codex/`, or `.gemini/`;
  those roots are agent-local settings only.
- Bundled resources (references, scripts, assets) go in subdirectories of the skill directory.

## SKILL.md format

```markdown
---
name: skill-name
description: When to trigger and what it does. Be specific and slightly pushy -- Claude undertriggers skills, so include concrete contexts. All "when to use" info goes in the description, not the body.
---

# Skill Title

Body: instructions the agent follows when the skill triggers.
Keep under 500 lines. For larger skills, use references/ for overflow.
```

Required frontmatter fields:
- `name` -- skill identifier (matches directory name)
- `description` -- triggering text. This is what Claude sees in its skill list to decide whether to load the skill. Include both what the skill does AND specific phrases/contexts that should trigger it.

Optional frontmatter:
- `user-invocable: true` -- lets users invoke with `/skill-name`
- `allowed-tools: Read, Grep, Bash` -- restrict which tools the skill can use
- `context: fork` -- run in a subagent instead of main context

## Progressive disclosure

Skills load in three tiers:
1. **Metadata** (name + description) -- always in context (~100 words)
2. **SKILL.md body** -- loaded when skill triggers (<500 lines ideal)
3. **Bundled resources** -- loaded on demand from references/ (unlimited size)

Keep SKILL.md lean. If approaching 500 lines, split detail into `references/` files and add clear pointers: "Read `references/advanced.md` for the full configuration reference."

## Adding a skill

1. `mkdir config/skills/<name>`
2. Write `config/skills/<name>/SKILL.md` with frontmatter + instructions
3. It's immediately available to both CLIs (live reload, no restart)

For community skills from `npx skills find` or skills.sh:
```bash
curl -sL https://raw.githubusercontent.com/<owner>/<repo>/main/skills/<name>/SKILL.md \
  -o config/skills/<name>/SKILL.md
```

## Removing a skill

`rm -rf config/skills/<name>` -- the source is gone and profile/agent injection
can no longer include it.

## When to split vs. bundle

- **Split** into separate skill directories when the skills have different trigger conditions. A debugging skill and a release skill should be separate -- they trigger on different user intents.
- **Bundle** into one skill with references/ when the content is one domain with multiple sub-topics. A frontend skill that covers Svelte patterns, chart library, and CSS conventions is one skill with optional reference files.

## Naming conventions

Skills are flat (one level under `config/skills/`). Nested subdirectories are
not valid skill roots. Use **prefix-based grouping** to organize related skills
into logical categories:

```
config/skills/
  dev-testing/SKILL.md          dev category -- testing
  dev-debugging/SKILL.md        dev category -- debugging
  dev-diagnostics/SKILL.md      dev category -- in-VM diagnostics
  build-images/SKILL.md         build category -- capsem-builder
  build-initrd/SKILL.md         build category -- initrd repack
  release-process/SKILL.md      release category
  release-docs/SKILL.md         release category -- site docs
  find-skills/SKILL.md          meta (no prefix needed)
  skill-creation/SKILL.md       meta
  organize-skills/SKILL.md      meta
```

Rules:
- Lowercase kebab-case: `dev-testing`, `build-images`
- Prefix is the category, suffix is the topic: `<category>-<topic>`
- Meta/standalone skills that don't belong to a category skip the prefix
- Name after the action or domain: what the skill helps you *do*
- Avoid generic names like `utils` or `helpers`

Current categories:
- `meta-*` -- skills about skills (find, create, organize)
- `dev-*` -- daily development (toolchain, testing, debugging, diagnostics)
- `build-*` -- building VM images and guest binaries
- `release-*` -- release process, CI, documentation site
- `frontend-*` -- frontend development (reserved)
