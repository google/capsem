# Lost Surface Audit

Date: 2026-06-12.

Branch under audit: `release/1.3-cleanup-pr-v2`.

## Immediate Finding

The top-level development skill surface was lost on this branch.

- `92fa3bd2 chore: establish true main snapshot` created the project dev skill
  library under top-level `skills/`.
- `5489ff10 chore: validate canonical skill library` moved that library into
  `config/skills/`.
- That move is wrong for the current architecture: `config/skills/` is
  product/profile payload, while top-level `skills/` plus `.codex/skills` is
  the project Codex/dev-agent operating manual.
- `origin/main` has `.codex/skills -> ../skills`; `HEAD` did not.

Correction rule: restore top-level `skills/` and `.codex/skills`, keep
`config/skills/` scoped to profile/product payload, and do not use
`config/skills/` for dev-agent instructions.

## Other Surfaces That Need Review

`git diff --name-status -M90% origin/main..HEAD` shows additional removed or
heavily reshaped surfaces. Some were intentional 1.3 burns, some were replaced,
and some need explicit accept/reject review before release:

- Agent symlinks: `.agents/skills`, `.claude/skills`, `.codex/skills`,
  `.cursor/skills`, `.gemini/skills`.
- Docs: profile/config/admin/security/observability/release pages were deleted
  while new 1.3 docs were added. Need a docs pass to ensure accepted contract
  pages replaced the old pages rather than silently removing needed guidance.
- Frontend: onboarding/provider/policy/settings components and tests were
  deleted while profile/security/plugin/stats route-backed surfaces were added.
  Need UI route coverage to prove every installed UI surface uses the new
  routes and no old/provider/setup theater remains.
- Site: `site/src/pages/faq.astro` was removed. Need accept/reject in the docs
  and marketing pass.
- Sprints: many historical sprint ledgers moved or disappeared relative to
  `origin/main`. Need preserve the active release ledgers and avoid losing
  evidence that still drives 1.3 recovery.
- Schemas/data/security-engine artifacts: many old policy/profile/security
  schema and benchmark artifacts are absent. Intentional burns must be
  documented; any current contract schema must exist under the new 1.3 names.

Release hold: do not call the branch clean until each bucket above is marked
accepted, restored, or intentionally burned in the tracker.
