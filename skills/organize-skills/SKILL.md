---
name: organize-skills
description: Use when creating, reorganizing, or maintaining the skills/ directory. Covers the shared skill layout conventions, directory structure, SKILL.md format, symlink architecture, and how to add or restructure skills so both Claude Code and Gemini CLI discover them.
---

# Organize Skills

This project uses a shared `skills/` directory at the repo root. Both Claude Code and Gemini CLI discover skills from it via symlinks -- one set of files, two consumers.

## Directory structure

```
skills/                          Canonical location (checked into git)
  <skill-name>/
    SKILL.md                     Required -- the skill itself
    references/                  Optional -- large docs loaded on demand
    scripts/                     Optional -- executable helpers
    assets/                      Optional -- templates, icons, etc.

.claude/skills -> ../skills      Claude Code symlink
.agents/skills -> ../skills      Gemini CLI symlink
```

Rules:
- One skill per directory. The directory name is the skill identifier.
- Every skill directory must contain a `SKILL.md` file. No other naming is discovered.
- Never put files directly in `.claude/skills/` or `.agents/skills/` -- those are symlinks to `skills/`.
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

1. `mkdir skills/<name>`
2. Write `skills/<name>/SKILL.md` with frontmatter + instructions
3. It's immediately available to both CLIs (live reload, no restart)

For community skills from `npx skills find` or skills.sh:
```bash
curl -sL https://raw.githubusercontent.com/<owner>/<repo>/main/skills/<name>/SKILL.md \
  -o skills/<name>/SKILL.md
```

## Removing a skill

`rm -rf skills/<name>` -- both CLIs stop seeing it immediately.

## When to split vs. bundle

- **Split** into separate skill directories when the skills have different trigger conditions. A debugging skill and a release skill should be separate -- they trigger on different user intents.
- **Bundle** into one skill with references/ when the content is one domain with multiple sub-topics. A frontend skill that covers Svelte patterns, chart library, and CSS conventions is one skill with optional reference files.

## Naming conventions

- Use lowercase kebab-case: `find-skills`, `skill-creation`, `organize-skills`
- Name after the action or domain: what the skill helps you *do*
- Avoid generic names like `utils` or `helpers`
