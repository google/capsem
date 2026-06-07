---
name: dev-skills
description: How AI agent skills work -- discovery, loading, triggering, format, and organization. Use when building Capsem's skills system, implementing skill discovery for guest AI agents, or understanding how Claude Code and Gemini CLI consume SKILL.md files. Covers the SKILL.md format, discovery mechanics, progressive disclosure, naming conventions, and lessons learned from setting up this project's skills.
---

# AI Agent Skills System

This documents everything we know about how skills work across Claude Code and Gemini CLI, learned from building and organizing this project's 18+ skills. This knowledge will inform Capsem's own skills system for guest AI agents.

## Discovery

### Claude Code
- Looks in `.claude/skills/` (project) and `~/.claude/skills/` (global)
- Discovers `<name>/SKILL.md` -- one level of nesting only
- Nested directories (e.g., `category/skill/SKILL.md`) are NOT discovered
- Symlinks work -- we use `.claude/skills -> ../skills` to share with Gemini
- Live reload on file change, no restart needed

### Gemini CLI
- Looks in `.agents/skills/` or `.gemini/skills/`
- Same `<name>/SKILL.md` format as Claude Code
- We use `.agents/skills -> ../skills` symlink

### What does NOT work
- Nested categories: `skills/dev/testing/SKILL.md` is not found by either CLI
- Files named anything other than `SKILL.md` in a directory are not discovered as skills
- Files directly in the skills root (not in a subdirectory) are not discovered

## SKILL.md format

```yaml
---
name: skill-name
description: When to trigger and what it does. Be specific and pushy -- Claude undertriggers.
---

# Skill Title

Instructions the agent follows when triggered.
```

### Frontmatter fields
- `name` (required) -- skill identifier, should match directory name
- `description` (required) -- this is the PRIMARY trigger mechanism. Claude sees name + description in its skill list and decides whether to load the full body. Everything about "when to use" goes here.
- `user-invocable: true` -- lets users invoke with `/skill-name`
- `allowed-tools` -- restrict which tools the skill can use
- `context: fork` -- run in a subagent

### Description is everything for triggering

Claude undertriggers skills by default. Descriptions must be:
- Specific about WHAT the skill does
- Explicit about WHEN to use it (list concrete contexts, phrases, file types)
- Slightly pushy -- "Use this whenever X, even if Y" style

Bad: "Frontend development guide"
Good: "Capsem frontend design system. Use when building UI components, styling views, working with the design system, choosing colors, or understanding the component library."

## Progressive disclosure

Three loading tiers:
1. **Metadata** (~100 words) -- name + description, always in context for every conversation
2. **SKILL.md body** (<500 lines ideal) -- loaded when skill triggers
3. **Bundled resources** (unlimited) -- `references/`, `scripts/`, `assets/` subdirs, loaded on demand

This means: keep SKILL.md lean. Put detailed wire formats, API docs, and large references in `references/` with clear pointers from the SKILL.md body.

## Organization: prefix-based grouping

Flat directory structure with naming convention for categories:

```
skills/
  dev-testing/SKILL.md          dev category
  dev-debugging/SKILL.md        dev category
  build-images/SKILL.md         build category
  release-process/SKILL.md      release category
  meta-find-skills/SKILL.md     meta category
```

Categories we use: `meta-*`, `dev-*`, `build-*`, `release-*`, `site-*`, `frontend-*`.

## Bundled resources pattern

```
skill-name/
  SKILL.md                      Main instructions (<500 lines)
  references/
    wire-format.md              Detailed protocol docs
    community-skill.md          Fetched from npx skills / GitHub
  scripts/
    helper.sh                   Executable automation
  assets/
    template.html               Templates, icons
```

Reference from SKILL.md with: "Read `references/wire-format.md` for the full protocol details."

## Community skills

The `npx skills` CLI (skills.sh) discovers community skills. To use one:

```bash
npx skills find <query>          # Search
# Then manually fetch and place:
curl -sL https://raw.githubusercontent.com/<owner>/<repo>/main/<path>/SKILL.md \
  -o skills/<name>/references/<topic>.md
```

We place community skills as references (not top-level SKILL.md) because:
- They're context for our skills, not standalone triggers
- Our SKILL.md provides the project-specific framing
- Community skills may have generic advice that conflicts with our conventions

Quality bar: prefer official sources (anthropics/, sveltejs/, google-gemini/) or 1K+ installs. Verify content before bundling.

## Global skills

Skills in `~/.claude/skills/` are available across all projects. We install meta skills globally:
- `meta-find-skills` -- discover community skills
- `meta-organize-skills` -- skill conventions
- `meta-skill-creation` -- create/iterate skills

## Lessons learned

1. **Nested directories don't work** for skill discovery. Use prefix naming instead.
2. **Description quality drives triggering accuracy.** Vague descriptions = skill never loads.
3. **Wire format docs belong in references/**, not in the main SKILL.md. Keep the body actionable.
4. **Write references from source code**, not from memory. API wire formats drift and memory gets stale.
5. **One skill per concern.** MCP and MITM proxy are separate skills even though both handle network traffic -- they have different trigger conditions.
6. **Cross-reference between skills** using "See dev-testing-vm for..." style pointers in the body.
7. **Skills load on demand** -- having 18 skills costs nothing when they're not triggered. Don't try to merge skills to save space.
8. **Both CLIs read the same format.** SKILL.md with YAML frontmatter works for Claude Code and Gemini CLI. No duplication needed.
