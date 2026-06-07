# Sprint: credential-pipeline

Plan approved in-session. Cross-reference `MASTER.md` for follow-up
sprints that this one scaffolds for.

## Why now

`crates/capsem-core/src/host_config.rs` hand-rolls detection for a
fixed subset. It hides what was tried, ignores `~/.capsem/user.toml`
as a valid source, does not cover skills or MCP servers. My Anthropic
key was missed during `just install`; Gemini credentials collapse
into a single badge in the UI.

## Direction

1. Detection must *generalize* — adding a new thing to detect should
   be TOML + spec, not Rust.
2. Reuse the existing settings-tree taxonomy (`ai.*`, `repository.*`,
   `vm.*`). Add exactly one new top-level group: `connectors`.
   Connector-shaped items (MCP servers, skills, and future Workspace
   / gcloud / GitHub OAuth) live there.
3. "Laxer" VM posture: do NOT bring host agent *settings/state* files
   (`settings_json`, `state_json`, `projects_json`) into VM defaults.
   Credentials still flow — API keys, Anthropic OAuth
   (`credentials_json`), Google ADC (`google_adc_json`). Dropping
   credential writeback would be a regression.
4. Ship `capsem detect` CLI + `GET /detect` endpoint first, then
   surface in UI: providers stay in ProvidersStep; inventory +
   future connectors live under a new Connectors section in Settings.

## Phase 0 recon (done)

- Anthropic on my Mac: key ships via `apiKeyHelper` in
  `~/.claude/settings.json` or login keychain — neither checked today.
- I have `GEMINI_API_KEY` env *and* gcloud ADC — two distinct creds.
- 4 Claude Code skills in `~/.claude/skills/`; 27+ MCP servers in
  `~/.claude.json` + `~/.claude/settings.json`.
- Python builder (Pydantic `extra='ignore'`) drops unknown TOML keys,
  so new `[[<slot>.sources]]` tables leave `defaults.json` and the
  generated mock TS unchanged.
- `detect_*` callsites live only in
  `crates/capsem/src/setup.rs:243` and
  `crates/capsem-service/src/main.rs:1717` (`/setup/detect`). Public
  surface to preserve: `HostConfig`, `DetectedConfigSummary`,
  `KeyValidation`, `detect()`, `detect_and_write_to_settings()`,
  `validate_api_key()`.
- New source kinds needed: `exec`, `exec_from_json`, `mac_keychain`
  (for Anthropic on macOS), plus `capsem_setting` (user-typed value
  counts as a source).

## Glossary — "writeback"

After detection finds a value, write it into `~/.capsem/user.toml`
via `policy_config::write_setting` so it persists as a normal user
setting. Today's `detect_and_write_to_settings` already does this for
API keys, git identity, SSH pubkey, github token, claude
`credentials_json`, google `adc_json`. This sprint preserves all of
those. A `writeback` bool per spec entry lets future slots opt out.

## Scope

**Scalar slots** (first-match-wins, writeback=yes):

```
ai.anthropic.api_key
ai.anthropic.claude.credentials_json
ai.google.api_key
ai.google.gemini.google_adc_json
ai.openai.api_key
repository.git.identity.author_name
repository.git.identity.author_email
repository.providers.github.token
repository.providers.gitlab.token
vm.environment.ssh.public_key
```

**Inventory slots** (enumerated, cross-agent, report-only):

```
connectors.mcp_servers     items = {name, agents: [...], origins: [...]}
connectors.skills          items = {name, agents: [...], origins: [...]}
```

**Connectors scaffold:** add a `connectors` top-level group to
`config/defaults.json`. Concrete connector settings (`connectors.google_workspace.*`,
`connectors.gcloud.*`, `connectors.github.*`) are each their own
follow-up sprint — this sprint puts the namespace in place so those
adds are zero-friction.

## Architecture

1. **Spec** — TOML `[[<slot_id>.sources]]` tables, embedded via
   `include_dir!` at build time.
   - `guest/config/ai/<provider>.toml` — per-provider scalars.
   - `guest/config/detect/host.toml` — `repository.*`, `vm.environment.*`.
   - `guest/config/detect/connectors.toml` — inventory + stubs for
     future connector specs.
2. **Walker** — pure fn in capsem-core:
   `detect(registry, ctx) -> Vec<DetectionReport>`.
   `ctx = { home: &Path, settings: &EffectiveSettings, os: TargetOs }`.
3. **Surface** — `capsem detect` CLI + `GET /detect`. UI splits:
   ProvidersStep + Settings > AI Providers for scalar `ai.*`, per-row
   "Why?" disclosure; new Settings > Connectors for inventory and
   future connector slots.

## Report shape (frozen)

```json
{
  "reports": [
    {
      "slot_id": "ai.anthropic.api_key",
      "kind": "scalar",
      "display_name": "Anthropic API Key",
      "selected": {"source_kind":"exec_from_json","source_name":"apiKeyHelper"},
      "writeback": true,
      "sources": [
        {"kind":"capsem_setting","name":"ai.anthropic.api_key","status":"not_found"},
        {"kind":"env","name":"ANTHROPIC_API_KEY","status":"not_found"},
        {"kind":"json_field","path":"~/.claude/settings.json","field":"apiKey","status":"not_found"},
        {"kind":"exec_from_json","path":"~/.claude/settings.json","field":"apiKeyHelper","status":"matched"},
        {"kind":"mac_keychain","service":"Claude Code-credentials","status":"skipped"},
        {"kind":"plain_file","path":"~/.anthropic/api_key","status":"skipped"}
      ]
    },
    {
      "slot_id": "connectors.mcp_servers",
      "kind": "inventory",
      "display_name": "MCP Servers",
      "items": [
        {"name":"github","agents":["claude_code"],"origins":[{"kind":"json_map","path":"~/.claude.json","field":"mcpServers","agent":"claude_code"}]},
        {"name":"capsem","agents":["claude_code","gemini_cli"],"origins":[...]}
      ],
      "sources": [
        {"kind":"json_map_enumerate","path":"~/.claude.json","field":"mcpServers","agent":"claude_code","status":"matched","count":27},
        {"kind":"json_map_enumerate","path":"~/.claude/settings.json","field":"mcpServers","agent":"claude_code","status":"matched","count":1},
        {"kind":"json_map_enumerate","path":"~/.gemini/settings.json","field":"mcpServers","agent":"gemini_cli","status":"not_found"}
      ]
    }
  ]
}
```

## Source kinds

Scalar (one string; first-match-wins; writeback if set):

| kind | params |
|---|---|
| `capsem_setting` | `setting_id` |
| `env` | `name` |
| `plain_file` | `path` |
| `json_field` | `path`, `field` |
| `json_contains` | `path`, `markers[]` |
| `exec` | `command`, `args[]?`, `timeout_ms` |
| `exec_from_json` | `path`, `field`, `timeout_ms` |
| `mac_keychain` | `service`, `account?` |

Inventory (list; dedupe by `name`; require `agent` tag):

| kind | params | item shape |
|---|---|---|
| `directory_enumerate` | `path`, `item_kind`, `agent` | `{name, path}` |
| `json_map_enumerate` | `path`, `field`, `agent` | `{name, config_snippet}` |

New source kinds later (YAML, plist, exec_jsonl) = handler addition,
not a schema change.

## CLI output

```
$ capsem detect
AI > Anthropic
  api_key                           ✓ found  exec apiKeyHelper (~/.claude/settings.json)
  claude.credentials_json           × none   (2 sources checked)
AI > Google
  api_key                           ✓ found  env GEMINI_API_KEY
  gemini.google_adc_json            ✓ found  file ~/.config/gcloud/application_default_credentials.json
AI > OpenAI
  api_key                           × none
Repository > Git
  identity.author_name              ✓ found  exec `git config --global user.name`
  identity.author_email             ✓ found  exec `git config --global user.email`
Repository > GitHub
  token                             ✓ found  exec `gh auth token`
VM > Environment > SSH
  public_key                        ✓ found  file ~/.ssh/id_ed25519.pub
Connectors
  mcp_servers                       28 items  (claude_code: 28, gemini_cli: 0)
  skills                            4 items   (claude_code: 4, gemini_cli: 0)
```

Flags: `--json`, `--slot <id>`, `--group <prefix>`, `-v/--verbose`,
`--why <slot_id>`.

## Files

**Create:**
- `crates/capsem-core/src/host_config/mod.rs` (split from
  `host_config.rs`; existing content moves verbatim).
- `crates/capsem-core/src/host_config/types.rs`
- `crates/capsem-core/src/host_config/spec.rs` (loader, `include_dir!`).
- `crates/capsem-core/src/host_config/walker.rs`
- `crates/capsem/src/detect.rs` (CLI subcommand).
- `guest/config/detect/host.toml`
- `guest/config/detect/connectors.toml` (inventory + placeholder
  stubs for google_workspace / gcloud / github).

**Modify:**
- `guest/config/ai/anthropic.toml` — append
  `[[ai.anthropic.api_key.sources]]` (6 sources inc
  `exec_from_json apiKeyHelper` + `mac_keychain`) +
  `[[ai.anthropic.claude.credentials_json.sources]]`.
- `guest/config/ai/google.toml` — same for `ai.google.api_key` and
  `ai.google.gemini.google_adc_json`.
- `guest/config/ai/openai.toml` — `ai.openai.api_key`.
- `config/defaults.json` — add `connectors` top-level group. Scope
  TBD during coding — see unresolved #3.
- `crates/capsem-core/src/host_config/mod.rs` — replace private
  `detect_*` fns with walker shim. `detect_host_config()` builds
  `HostConfig` from scalar reports; `detect_and_write_to_settings()`
  writes only scalars with `writeback = true` AND
  `selected != capsem_setting` (no self-write).
- `crates/capsem/src/main.rs` — register `Detect` subcommand.
- `crates/capsem-service/src/main.rs` — add `GET /detect`. Keep
  `/setup/detect` for one release.
- `frontend/src/lib/types/onboarding.ts` — `DetectionReport` types.
- `frontend/src/lib/api.ts` — `getDetectionReports()`.
- `frontend/src/lib/components/onboarding/ProvidersStep.svelte` —
  drop hardcoded `providerDefs`; iterate scalar reports under `ai.*`;
  per-row "Why?" disclosure.
- `frontend/src/lib/components/shell/SettingsPage.svelte` — provenance
  badges next to detectable leaves + new Connectors section rendering
  the two inventory slots (counts + expandable lists with per-item
  agents).

**Delete:**
- All five `detect_*` private fns + their unit tests in
  `host_config.rs`.
- `DETECT_SETTING_MAP` / `DETECT_FILE_MAP` constants — writeback
  metadata lives on the spec.

## Reusable code

- `extract_json_string_field` (host_config.rs:458) → `json_field`.
- `non_empty_env`, `read_key_file` → `env`, `plain_file`.
- `policy_config::load_settings_files` + `write_setting` →
  `capsem_setting` + writeback.
- Existing `Command` pattern → `exec` / `exec_from_json` / `mac_keychain`.
- `ProvidersStep.svelte::findLeaf(tree, id)` → badge helper.

## Tests

Rust: per source kind (match/not_found/invalid/unsupported via
injected `DetectContext`); scalar walker first-match-wins; inventory
walker dedupe + origins; `mac_keychain` round-trip under
`#[cfg(target_os="macos")]` + `CAPSEM_KEYCHAIN_TESTS=1`; golden JSON
for `DetectionReport`; backwards-compat for
`detect_and_write_to_settings`.

Python (builder): existing `test_defaults_json_not_stale` +
`test_mock_ts_not_stale` stay green.

CLI: `tests/capsem-install/test_detect_cli.py` — `--json`, `--group`,
`--slot`, `--why`, `-v`.

Frontend: vitest for badge status→colour + disclosure rendering.
Chrome DevTools MCP screenshots: ProvidersStep per-kind rows,
Settings badges, Connectors section.

## Verification

```bash
cargo test -p capsem-core host_config::
cargo test -p capsem --bin capsem detect
cargo clippy --workspace --all-targets

./target/release/capsem detect
./target/release/capsem detect --why ai.anthropic.api_key
./target/release/capsem detect --group connectors -v
./target/release/capsem detect --json | jq '.reports[] | {slot_id, kind}'

just smoke
curl -s --unix-socket ~/.capsem/run/service.sock \
  -H "Authorization: Bearer $(cat ~/.capsem/run/gateway.token)" \
  http://localhost/detect | jq '.reports | length'

cd frontend && pnpm run check && npx vitest run
just test
```

Exit criteria:
- `capsem detect` reports my Anthropic key as `found` with a truthful
  source (likely `exec_from_json apiKeyHelper`).
- Gemini shows two scalar rows, both written back to settings.
- `connectors.mcp_servers` ≥27 items; `connectors.skills` 4.
- `config/defaults.json` has a `connectors` top-level group.
- Every old `detect_*` fn gone.
- Adding a new scalar slot = TOML edit only; no Rust.

## Execution order

Three commits, each self-contained with CHANGELOG + green tests:

1. `feat(core): declarative detection + connectors scaffold` —
   types, loader, walker (scalar + inventory), all source kinds,
   dedupe, unit tests. Provider TOMLs updated + new `host.toml` +
   `connectors.toml`. `connectors` group in `defaults.json`. Old
   `detect_*` deleted.
2. `feat(cli,service): capsem detect + /detect endpoint` — subcommand
   with all flags, service handler, `/setup/detect` compat wrapper,
   Python smoke test.
3. `feat(frontend): provenance badges + Connectors inventory` —
   types, API, wizard rewrite, Settings badges + Connectors section.
   Vitest + visual verification.

## Non-goals (scaffolded as follow-up sprints)

- `followup-google-workspace.md` — gws CLI connector.
- `followup-gcloud.md` — gcloud SDK / ADC connector.
- `followup-github-oauth.md` — GitHub OAuth beyond the PAT.
- `followup-inventory-injection.md` — inject detected skills / MCP
  servers into VMs.

## Unresolved

1. **`exec_from_json` shape**: if value contains spaces, run via
   `sh -c`; otherwise exec directly. Documented inline in handler.
2. **Inventory dedupe tiebreaker**: same MCP in both `~/.claude.json`
   and `~/.claude/settings.json` — one item, both origins, first
   declared source wins for `config_snippet`.
3. **`inventory_readonly` setting type vs. Connectors-reads-/detect
   directly**: lean toward the second (empty `connectors` group in
   `defaults.json`, UI reads reports). Confirm during coding.
