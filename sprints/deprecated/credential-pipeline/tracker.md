# Sprint: credential-pipeline -- tracker

See `plan.md` for full context. See `MASTER.md` for follow-up sprints.

## Phase 0: Reconnaissance  [DONE]

- [x] Walk every callsite of `detect_*`. Producers: `capsem/src/setup.rs:243`
      + `capsem-service/src/main.rs:1717`. Public surface to preserve:
      `HostConfig`, `DetectedConfigSummary`, `KeyValidation`, `detect()`,
      `detect_and_write_to_settings()`, `validate_api_key()`.
- [x] Builder-side `src/capsem/builder/config.py::_load_ai_providers`
      uses Pydantic `extra='ignore'`. Unknown TOML sections silently
      stripped. `config/defaults.json` and
      `frontend/src/lib/mock-settings.generated.ts` do not change.
- [x] My Mac diagnosis:
      - Anthropic: ANTHROPIC_API_KEY unset; ~/.claude/settings.json
        present but no literal `apiKey` field — Claude Code on macOS
        uses `apiKeyHelper` (script path in that JSON) or login
        keychain. New sources needed: `exec_from_json`, `mac_keychain`.
      - Gemini: `GEMINI_API_KEY=AIza...` in env AND ADC file present
        — two distinct credentials, UI must show two rows.
      - Skills: 4 in `~/.claude/skills/`. MCP: 27+ entries in
        `~/.claude.json` + `~/.claude/settings.json`. Both invisible
        today.
- [x] Inventory: `capsem_setting` must be a first-class source kind
      so user-typed values show up in the report with correct
      provenance.

## Phase 1: Commit 1 — core walker

`feat(core): declarative detection + connectors scaffold`

- [ ] Split `crates/capsem-core/src/host_config.rs` into a module
      directory. Existing content moves to `mod.rs` verbatim. Make
      helpers `pub(super)` so submodules can reuse:
      `extract_json_string_field`, `non_empty_env`, `read_key_file`.
- [ ] `host_config/types.rs` — `DetectionReport` (scalar +
      inventory variants), `SourceOutcome`, `DetectionStatus`,
      `SourceSpec`, `Agent`, `InventoryItem`, `Origin`.
- [ ] `host_config/spec.rs` — loader + `DetectRegistry` embedded via
      `include_dir!`. Resolves `~` in paths. Tolerates unknown TOML
      keys.
- [ ] `host_config/walker.rs` — `detect(registry, ctx)`. One private
      handler per source kind. Inventory dedupe by `name`, merge
      `agents` + `origins`.
- [ ] Unit tests per source kind (match / not_found / invalid /
      unsupported) via injected `DetectContext`.
- [ ] Inventory walker tests: dedupe, origins carried, per-source
      counts.
- [ ] `mac_keychain` round-trip under `#[cfg(target_os="macos")]`
      + `CAPSEM_KEYCHAIN_TESTS=1`; cheap not-found smoke runs
      everywhere.
- [ ] Golden JSON test pinning `DetectionReport` shape.
- [ ] Append `[[<slot>.sources]]` tables to `guest/config/ai/*.toml`
      for `ai.anthropic.api_key`, `ai.anthropic.claude.credentials_json`,
      `ai.google.api_key`, `ai.google.gemini.google_adc_json`,
      `ai.openai.api_key`.
- [ ] `guest/config/detect/host.toml` — `repository.*` +
      `vm.environment.ssh.public_key`.
- [ ] `guest/config/detect/connectors.toml` —
      `connectors.mcp_servers` + `connectors.skills` +
      placeholder comments for workspace / gcloud / github.
- [ ] `config/defaults.json` — add `connectors` top-level group
      (keep scope minimal per unresolved #3 in plan.md).
- [ ] Replace private `detect_*` fns with walker calls;
      `detect_host_config()` builds `HostConfig` from scalar reports;
      `detect_and_write_to_settings()` writes only scalars with
      `writeback = true` and `selected != capsem_setting`.
- [ ] Delete `DETECT_SETTING_MAP` / `DETECT_FILE_MAP` constants.
- [ ] Delete old `detect_anthropic_key` / `detect_google_key` /
      `detect_openai_key` / `detect_claude_oauth` / `detect_google_adc`
      and their unit tests.
- [ ] `cargo test -p capsem-core` green.
- [ ] `cargo clippy -p capsem-core --all-targets` green.
- [ ] `just test` Python portion green (confirms Pydantic
      `extra='ignore'` still drops new sections).
- [ ] `CHANGELOG.md` entry under Unreleased > Changed.
- [ ] Commit.

## Phase 2: Commit 2 — CLI + service endpoint

`feat(cli,service): capsem detect + /detect endpoint`

- [ ] `crates/capsem/src/detect.rs` — `run_detect(opts)` formatter
      with `--json`, `--slot`, `--group`, `--verbose`, `--why`.
      Resolves display names via settings tree.
- [ ] `crates/capsem/src/main.rs` — register `Detect` subcommand
      with the flags above.
- [ ] `crates/capsem-service/src/main.rs` — `handle_detect()` at
      `GET /detect` returns `Vec<DetectionReport>`. `GET /setup/detect`
      becomes a thin wrapper that also includes the legacy
      `DetectedConfigSummary` for one release.
- [ ] `tests/capsem-install/test_detect_cli.py` — `--json` returns
      valid JSON; `--group ai.anthropic` narrows; `--slot
      connectors.skills -v` lists items; `--why ai.anthropic.api_key`
      prints source outcomes.
- [ ] `just smoke` green. `capsem detect` on my Mac matches exit
      criteria (Anthropic found, Gemini two rows, MCP ≥27, skills 4).
- [ ] `CHANGELOG.md` entry under Unreleased > Added.
- [ ] Commit.

## Phase 3: Commit 3 — Frontend

`feat(frontend): provenance badges + Connectors inventory`

- [ ] `frontend/src/lib/types/onboarding.ts` — `DetectionReport`,
      `ScalarReport`, `InventoryReport`, `SourceOutcome`,
      `DetectionStatus`.
- [ ] `frontend/src/lib/api.ts` — `getDetectionReports()` on
      `/detect`.
- [ ] `ProvidersStep.svelte` — drop hardcoded `providerDefs`;
      iterate scalar reports under `ai.*`; per-row "Why?" disclosure
      listing sources with status icons.
- [ ] `SettingsPage.svelte` — provenance badge next to each
      detectable leaf (small dot + tooltip). New Connectors section
      rendering the two inventory slots: count summary, expandable
      list of items with per-item agents.
- [ ] Vitest: badge status→colour mapping, disclosure rendering.
- [ ] `pnpm run check` + `npx vitest run` green.
- [ ] Chrome DevTools MCP verification: `just ui`, navigate to
      wizard ProvidersStep (Anthropic two rows), Settings > AI
      (badges), Settings > Connectors (expand mcp_servers + skills).
      Screenshot each.
- [ ] `CHANGELOG.md` entry under Unreleased > Added.
- [ ] Commit.

## Phase 4: Testing gate + cleanup

- [ ] `just test` full pass (Rust + frontend + Python + integration).
- [ ] `just run "capsem-doctor"` green.
- [ ] Walk CHANGELOG entries across the three commits for clarity.
- [ ] No debug prints / TODO / temporary hacks left.
- [ ] Review whether `/simplify` needs to run on core/walker.rs.

## Notes / discoveries (append as we go)

- Public surface to preserve:
  `HostConfig`, `DetectedConfigSummary`, `KeyValidation`, `detect()`,
  `detect_and_write_to_settings()`, `validate_api_key()`.
- Builder safety: Pydantic `extra='ignore'` already confirmed via
  Phase 0 — new TOML sections are invisible to the generator, so no
  goldens move.
- New source kinds discovered during recon: `exec_from_json` (run
  a script named in a JSON field) and `mac_keychain` (wrap
  `security find-generic-password -w`). Without these the current
  Anthropic detection miss can't be fixed.
- Settings tree as the display taxonomy — no parallel "domains"
  concept introduced, per direction from user.
- Writeback preserved exactly as-is for all detected credentials,
  matching current behavior.
