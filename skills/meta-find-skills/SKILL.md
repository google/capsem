---
name: find-skills
description: "Searches the skills.sh registry, compares options by install count and source reputation, and runs installation commands for agent skills. Use when users ask 'how do I do X', 'find a skill for X', 'is there a skill that can...', or want to extend agent capabilities with installable skills."
---

# Find Skills

Discover and install skills from the open agent skills ecosystem via the Skills CLI (`npx skills`).

## CLI Reference

```bash
npx skills find [query]              # Search by keyword
npx skills add <owner/repo@skill>    # Install from GitHub
npx skills add <owner/repo@skill> -g # Install globally (user-level)
npx skills check                     # Check for updates
npx skills update                    # Update all installed skills
```

Browse all skills: https://skills.sh/

## Workflow

### 1. Search

Map the user's request to a search query and check the [skills.sh leaderboard](https://skills.sh/) first for well-known skills. If no match, search via CLI:

```bash
npx skills find react performance
npx skills find pr review
npx skills find changelog
```

### 2. Verify Quality

Before recommending any skill, check:

| Signal | Threshold |
|--------|-----------|
| Install count | Prefer 1K+. Be cautious under 100. |
| Source reputation | Official orgs (`vercel-labs`, `anthropics`, `microsoft`) over unknown authors. |
| GitHub stars | Repos with <100 stars warrant extra scrutiny. |

### 3. Present and Install

Show the user: skill name, what it does, install count, source, and install command.

```bash
npx skills add vercel-labs/agent-skills@react-best-practices -g -y
```

### 4. Verify Installation

After installing, confirm the skill loaded correctly:

```bash
# Check the skill directory exists
ls .claude/skills/ | grep <skill-name>

# If install fails: retry without -y to see prompts, check network, verify the owner/repo@skill path is correct
npx skills add <owner/repo@skill>
```

## When No Skills Are Found

If no relevant skill exists:

1. Tell the user no match was found
2. Offer to help directly with general capabilities
3. Suggest creating a custom skill: `npx skills init my-skill-name`
