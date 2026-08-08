# Follow-up sprint: connectors.github

**Prereq:** credential-pipeline sprint merged.

## What

GitHub as a connector — beyond the existing
`repository.providers.github.token`. The PAT already covers push-over-HTTPS
for `git`. This sprint adds the richer `gh` CLI OAuth flow so users
can actually run GitHub-API-backed tooling inside the VM (gh CLI,
octokit, etc.) without minting a PAT by hand.

## Scope

New settings under `connectors.github`:

```
connectors.github.allow            bool,  default false (gated on user opt-in)
connectors.github.oauth_token      apikey (writeback yes, from `gh auth token`)
connectors.github.user             text,  detected via `gh api user -q .login`
connectors.github.hosts_yml        file   (writeback yes, full ~/.config/gh/hosts.yml)
connectors.github.domains          text,  default "*.github.com, *.githubusercontent.com"
connectors.github.install.*        gh CLI install manifest
```

The existing `repository.providers.github.token` is a separate concern
(git push) and stays where it is. Both can co-exist on the same host.

Detection spec tables (replace placeholder stub):

```toml
[[connectors.github.oauth_token.sources]]
kind = "exec"
command = "gh"
args = ["auth", "token"]
timeout_ms = 2000

[[connectors.github.oauth_token.sources]]
kind = "capsem_setting"
setting_id = "connectors.github.oauth_token"

[[connectors.github.hosts_yml.sources]]
kind = "plain_file"
path = "~/.config/gh/hosts.yml"

[[connectors.github.user.sources]]
kind = "exec"
command = "gh"
args = ["api", "user", "-q", ".login"]
timeout_ms = 2000
```

VM install manifest: gh CLI via package manager. Auth blob injected
at `/root/.config/gh/hosts.yml` inside the guest.

## Unresolved

- YAML source support. `hosts.yml` is YAML — we can detect existence
  with `plain_file` and just ship the whole blob, no parser needed.
  If we later want to enumerate hosts or check scopes, we'll add a
  `yaml_field` source kind.
- Relationship with `repository.providers.github.token`: the PAT is
  for git push; the OAuth token is for API calls. Both fine to have.

## Exit criteria

- `capsem detect` shows `connectors.github.oauth_token` as `found`
  when `gh auth status` is green on the host.
- `gh api user` inside the VM returns the user's github login.
- Settings > Connectors > GitHub has token, user, and host-config
  fields.
