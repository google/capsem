# Sprint: credential-pipeline -- tracker

See `plan.md` for context, scope, and exit criteria.

## Tasks

### Phase 0: Reconnaissance

- [ ] Read all existing `guest/config/ai/*.toml` and note the current
      `[<provider>.api_key]` + `[<provider>.files.*]` shape so the
      `[<provider>.detect]` addition lives alongside, not replaces.
- [ ] Walk every callsite of the `detect_*` functions in `host_config.rs`
      (service `/setup/detect`, CLI setup steps, MCP export). Record the
      public surface we must keep intact.
- [ ] Read `src/capsem/builder/` entry points (providers.py, the thing
      that consumes `guest/config/ai/*.toml`) and confirm it tolerates
      unknown TOML keys -- the new `[detect]` section must not break the
      builder before step 4.
- [ ] Confirm on my Mac: which sources do I actually have for Anthropic
      right now? Check `ANTHROPIC_API_KEY` env, `~/.claude/settings.json`,
      `~/.anthropic/api_key`. One of them should explain why the wizard
      missed it.
- [ ] Read `gws` docs / GitHub releases page to confirm:
      auth file paths, encryption-key behavior, which env vars override.

### Phase 1: Landing gear -- spec loader + walker (no behavior change)

- [ ] New `crates/capsem-core/src/host_config/detect_spec.rs`:
      `DetectSpec`, `SourceSpec`, `SourceKind` (Env / PlainFile /
      JsonField / JsonContains), parser for `[<provider>.detect]`.
- [ ] `DetectionReport`, `SourceOutcome`, `DetectionStatus`. Serializable.
- [ ] Walker: `fn walk(spec: &DetectSpec, home: &Path) -> DetectionReport`.
      Pure function, no I/O outside the home dir passed in. Fully
      unit-testable.
- [ ] Unit tests per source kind: env matched / env empty /
      plain_file missing / plain_file whitespace-only /
      json_field missing / json_field present / json_contains matched /
      json_contains markers-missing.
- [ ] Load the three existing provider TOMLs into `DetectSpec` from disk
      without adding `[detect]` sections yet -- confirm round-trip.
- [ ] `cargo test -p capsem-core detect_spec::` green.

### Phase 2: Switch over (replace `detect_*`, delete old)

- [ ] Add `[anthropic.detect]`, `[google.detect]`, `[openai.detect]` to
      the three existing TOMLs. Two credential kinds for Anthropic
      (api_key, claude_oauth); three for Google (api_key, adc,
      -- workspace added later); one for OpenAI.
- [ ] Replace `detect_anthropic_key` / `detect_google_key` / etc. with
      calls to `walk(load_spec(provider))`. Keep `HostConfig` shape
      identical so no downstream breaks.
- [ ] Port existing unit tests to drive the walker with fixture home
      dirs. Delete the `detect_*` fn-level tests.
- [ ] `cargo test -p capsem-core` green.
- [ ] `cargo test -p capsem --bin capsem` green.
- [ ] `just smoke` green on my Mac. Re-verify my Anthropic key is now
      detected (Phase 0 diagnosis will say which source).
- [ ] **Commit:** `refactor(host_config): declarative credential detection`

### Phase 3: Inspection surface

- [ ] `GET /setup/detect` response adds `reports: DetectionReport[]`
      alongside the existing `DetectedConfigSummary`. Shape frozen.
- [ ] `capsem setup --explain` -- new subcommand flag that walks the spec
      and prints the report table to stdout. Exits 0 whether or not
      anything was found.
- [ ] `ProvidersStep.svelte`: drop `providerDefs` array, iterate the
      reports. Badge per credential kind. "Why?" disclosure shows the
      source list.
- [ ] Vitest covering the disclosure rendering.
- [ ] Screenshot via chrome-devtools MCP: verify the new per-kind layout.
- [ ] **Commit:** `feat(setup): inspectable credential detection reports`

### Phase 4: Google Workspace provider

- [ ] New `guest/config/ai/google_workspace.toml` with:
      - `[google_workspace]` metadata
      - `[google_workspace.cli]` (key=gws, version_command)
      - `[google_workspace.api_key]` NOT present -- workspace is not
        api-key based. Instead:
      - `[google_workspace.detect]` with sources:
        env `GOOGLE_WORKSPACE_CLI_TOKEN` (kind=env),
        env `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` (kind=env_path),
        file `~/.config/gws/<keyring-blob>` (path TBD -- confirm from docs)
      - `[google_workspace.network]` with `*.googleapis.com` and
        workspace-specific domains (`*.google.com`? TBD).
      - `[google_workspace.install]` brew manager, package
        `googleworkspace-cli`. npm fallback `@googleworkspace/cli` for
        Linux guests.
      - `[google_workspace.files.*]` to inject the detected credential
        into `/root/.config/gws/` inside the guest.
- [ ] `src/capsem/builder/` -- verify multi-provider-per-family works
      (google + google_workspace side-by-side). Extend if needed.
- [ ] `just build-assets` green. Rootfs contains gws binary.
- [ ] `just shell` boots, `gws --help` works, `gws drive files list`
      succeeds against the detected credential (or fails with a clean
      auth error if no creds detected on host).
- [ ] Frontend: Google provider row now shows three credential kinds
      (api_key, adc, workspace_oauth).
- [ ] **Commit:** `feat(providers): add Google Workspace via gws CLI`

### Phase 5: Polish + docs

- [ ] Settings > AI Providers: same "Why?" disclosure, so post-install
      inspection doesn't require re-opening the wizard.
- [ ] Update `skills/dev-capsem/SKILL.md` with a pointer to
      `guest/config/ai/` as the single source of truth for providers.
- [ ] (Optional) `docs/` page: "Adding an AI provider" -- walks through
      the TOML spec end-to-end.
- [ ] `just test` full pass.
- [ ] `just run "capsem-doctor"` green.
- [ ] **Commit:** `docs(providers): spec + inspection, closing sprint`

## Notes

- The guest-side already being declarative is what makes this sprint
  tractable -- half the work is shaped by the existing per-provider
  TOMLs. We're adding a `[detect]` section, not a new system.
- `DetectionReport` shape needs to be frozen early (step 1) because the
  UI and CLI consumers diverge fast if it changes mid-sprint.
- gws auth details are partly TBD -- Phase 0 must confirm the actual
  paths from the gws docs, not guess.
- Keep the existing `detect_and_write_to_settings` signature intact; it
  is called from settings and test paths we do not want to chase.
