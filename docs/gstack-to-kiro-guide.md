# Adapting Gstack Skills for Kiro/Cline

## Overview

**Good news:** Kiro/Cline uses the **same SKILL.md standard** as Claude Code. Gstack skills can be adapted to work in Kiro with some modifications — mainly stripping out Claude Code-specific infrastructure.

## How Kiro Skills Work

Kiro discovers skills from SKILL.md files placed in specific directories:

| Location | Scope |
|----------|-------|
| `~/.kiro/skills/<skill-name>/SKILL.md` | Global — available in all projects |
| `<project>/.kiro/skills/<skill-name>/SKILL.md` | Project-local — only this repo |

Each skill is a folder containing a `SKILL.md` file with:
1. **YAML frontmatter** — `name` and `description` (required)
2. **Markdown body** — the detailed instructions the AI follows when the skill is activated

### Minimal SKILL.md Format

```markdown
---
name: my-skill
description: |
  One-paragraph description of what this skill does and when to use it.
---

# Skill Title

## Step 1: Do something
...instructions...

## Step 2: Do something else
...instructions...
```

## What Needs to Change from Gstack → Kiro

### 1. Remove the Bash Preamble (REQUIRED)

Gstack skills have a large `## Preamble (run first)` section with bash commands that:
- Check for gstack updates
- Track sessions and telemetry
- Detect repo mode
- Load learnings

**This entire section must be removed.** Kiro doesn't execute preamble scripts.

### 2. Remove Template Variables (REQUIRED)

Gstack `.tmpl` files use template variables like:
- `{{PREAMBLE}}` — the bash preamble
- `{{BASE_BRANCH_DETECT}}` — git branch detection logic
- `{{LEARNINGS_LOG}}` — learnings persistence
- `{{LEARNINGS_SEARCH}}` — learnings lookup
- `{{SCOPE_DRIFT}}` — scope drift detection
- `{{REVIEW_ARMY}}` — parallel review agents
- `{{ADVERSARIAL_STEP}}` — adversarial testing
- `{{CONFIDENCE_CALIBRATION}}` — confidence scoring

Replace these with inline equivalents or remove them. The `.tmpl` files show where these go; the generated `SKILL.md` files show what they expand to.

### 3. Strip Gstack Binary References (REQUIRED)

Remove or replace references to gstack binaries like:
- `~/.claude/skills/gstack/bin/gstack-*`
- `gstack-config`, `gstack-telemetry-log`, `gstack-learnings-log`, etc.

### 4. Simplify `allowed-tools` (RECOMMENDED)

Gstack uses Claude Code tool names (`Read`, `Grep`, `Glob`, `Bash`, `Edit`, `Write`, `Agent`, `AskUserQuestion`, `WebSearch`). Kiro doesn't use this field — you can remove it or keep it as documentation.

### 5. Remove `preamble-tier` and `benefits-from` (CLEANUP)

These are gstack-specific metadata. Remove them from the frontmatter.

### 6. Update File Paths (REQUIRED)

Replace `~/.claude/skills/gstack/` paths with appropriate Kiro paths or remove them entirely.

## Step-by-Step Adaptation Process

### Quick Method (per skill)

1. **Start from the SKILL.md.tmpl** (not the generated SKILL.md) — it's cleaner
2. **Keep the frontmatter** but simplify to just `name` and `description`
3. **Delete** the `{{PREAMBLE}}` line and any `{{TEMPLATE_VAR}}` lines
4. **Keep** all the actual instruction content (the philosophy, steps, rules, formatting)
5. **Remove** bash blocks that reference `gstack-*` binaries
6. **Save** as `~/.kiro/skills/<skill-name>/SKILL.md`

### Batch Method (all skills)

```bash
# 1. Clone gstack
git clone --depth 1 https://github.com/garrytan/gstack.git /tmp/gstack

# 2. For each skill, copy the SKILL.md and clean it up
# (See the automated script below)
```

## Which Skills Are Worth Porting?

### High Value (pure prompt engineering — easy to port)
These skills are mostly instructions with minimal gstack infrastructure:

| Skill | What it does |
|-------|-------------|
| `plan-ceo-review` | CEO/founder-mode plan review with 4 modes (expand/selective/hold/reduce scope) |
| `plan-eng-review` | Engineering architecture review — locks in design before coding |
| `plan-design-review` | Design/UX review of plans |
| `office-hours` | YC-style brainstorming — forcing questions, premise challenges |
| `design-consultation` | Build a design system from scratch |
| `review` | Pre-landing PR review (SQL safety, race conditions, etc.) |
| `retro` | Sprint retrospective with metrics |
| `investigate` | Debug investigation with auto-freeze |
| `document-release` | Auto-update all docs to match code changes |
| `cso` | Security officer — OWASP + STRIDE audits |

### Medium Value (need some adaptation)
These reference gstack binaries or browser infrastructure:

| Skill | What it does | Adaptation needed |
|-------|-------------|-------------------|
| `ship` | Create PR with tests and coverage | Remove gstack-review-log references |
| `qa` | Open browser and test staging | Needs Playwright setup separately |
| `careful` | Warn before destructive commands | Uses git hooks — needs manual setup |
| `freeze/unfreeze` | Lock edits to one directory | Uses gstack state files |

### Low Value for Kiro (heavily dependent on gstack infrastructure)
| Skill | Why |
|-------|-----|
| `browse` / `connect-chrome` | Requires gstack's Playwright server |
| `codex` | Requires OpenAI Codex CLI |
| `gstack-upgrade` | Self-update mechanism |
| `canary` | Requires deployment infrastructure |
| `setup-deploy` | Requires deployment infrastructure |

## Example: Ported `plan-ceo-review` Skill

See `~/.kiro/skills/plan-ceo-review/SKILL.md` for a working example.

The original skill is ~3000 lines. The ported version keeps all the valuable review methodology while removing ~200 lines of gstack infrastructure (preamble, telemetry, binary calls).

## Tips

1. **Use `.tmpl` files as source** — they're the canonical version and show template boundaries clearly
2. **The core value is in the prompts** — the detailed instructions, philosophy, and review methodology are what make gstack powerful, and they transfer directly
3. **Test incrementally** — port one skill, try it, then port more
4. **Keep the formatting rules** — gstack's numbered issues + lettered options format works great with AI agents
5. **The mode tables are gold** — the comparison tables (like EXPANSION vs HOLD SCOPE) are pure prompt engineering and work perfectly in Kiro
