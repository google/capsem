# Follow-up sprint: connectors.google_workspace

**Prereq:** credential-pipeline sprint merged (`plan.md`/`tracker.md`
in this directory).

## What

Add Google Workspace as the first `connectors.*` entry. Users who
authenticate gws on the host get it automatically wired into their
VMs: `gws` CLI installed, auth blob injected, workspace network
domains added to the allow list.

## Why now (to be filled at kickoff)

- User interest signal / demand to land this TBD.
- Prereq: credential-pipeline has shipped and `connectors` namespace
  exists in `config/defaults.json`.

## Scope

New settings under `connectors.google_workspace`:

```
connectors.google_workspace.allow            bool,  default false
connectors.google_workspace.token            apikey (writeback yes)
connectors.google_workspace.credentials_json file   (writeback yes)
connectors.google_workspace.domains          text,  default "*.googleapis.com, *.google.com"
connectors.google_workspace.install.*        CLI install manifest
```

Detection spec tables in `guest/config/detect/connectors.toml`
(replace the placeholder stub):

```toml
[[connectors.google_workspace.token.sources]]
kind = "env"
name = "GOOGLE_WORKSPACE_CLI_TOKEN"

[[connectors.google_workspace.token.sources]]
kind = "capsem_setting"
setting_id = "connectors.google_workspace.token"

[[connectors.google_workspace.credentials_json.sources]]
kind = "env"
name = "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE"

[[connectors.google_workspace.credentials_json.sources]]
kind = "plain_file"
path = "~/.config/gws/.encryption_key"   # TBD -- verify against gws docs
```

VM install manifest (analogous to `guest/config/ai/google.toml`):

- `install.manager = "brew"` on macOS, fall back to
  `npm install -g @googleworkspace/cli` elsewhere.
- File injection: auth blob landed at `/root/.config/gws/`.

## Unresolved (resolve at kickoff)

- Actual on-disk paths for gws credential storage (OS keyring fallback
  file location). Read the gws docs + spike a test install.
- Whether to detect via `gws auth status --json` (exec source) rather
  than poking at encrypted files. Likely yes if the command exists.
- Network domains beyond `*.googleapis.com` required for Workspace
  APIs (Drive, Calendar, Gmail, Chat).

## Exit criteria

- `capsem detect` shows `connectors.google_workspace.token` (and/or
  `credentials_json`) as `found` when gws is authenticated on host.
- `capsem shell` boots a VM where `gws drive files list` succeeds.
- Settings > Connectors > Google Workspace has a standard allow
  toggle, credential field, and domain list.
- No changes to `crates/capsem-core/src/host_config/` beyond whatever
  new source kinds the gws credential shape demands (should be zero
  if `env` + `exec` + `plain_file` cover it).
