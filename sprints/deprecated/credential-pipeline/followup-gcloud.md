# Follow-up sprint: connectors.gcloud

**Prereq:** credential-pipeline sprint merged.

## What

gcloud SDK as a first-class connector, separate from the Gemini API
key. Users with gcloud authenticated on the host get gcloud installed
in the VM with their ADC injected, ready to hit GCP APIs.

Note: `ai.google.gemini.google_adc_json` already detects ADC for the
Gemini API. This connector covers the *gcloud CLI* usage path which
wants more than just the ADC file (account metadata, active project,
config directory).

## Scope

New settings under `connectors.gcloud`:

```
connectors.gcloud.allow            bool,  default false
connectors.gcloud.credentials_json file   (writeback yes, shares value with ai.google.gemini.google_adc_json if both detected)
connectors.gcloud.active_project   text,  detected via `gcloud config get project`
connectors.gcloud.account          text,  detected via `gcloud config get account`
connectors.gcloud.domains          text,  default "*.googleapis.com, *.cloud.google.com"
connectors.gcloud.install.*        CLI install manifest
```

Detection spec tables (replace placeholder stub in
`guest/config/detect/connectors.toml`):

```toml
[[connectors.gcloud.credentials_json.sources]]
kind = "plain_file"
path = "~/.config/gcloud/application_default_credentials.json"

[[connectors.gcloud.active_project.sources]]
kind = "exec"
command = "gcloud"
args = ["config", "get", "project"]
timeout_ms = 2000

[[connectors.gcloud.account.sources]]
kind = "exec"
command = "gcloud"
args = ["config", "get", "account"]
timeout_ms = 2000
```

VM install manifest: Google Cloud SDK install script or equivalent
package manager entry. File injection mirrors Workspace.

## Unresolved

- Dedupe policy between `ai.google.gemini.google_adc_json` and
  `connectors.gcloud.credentials_json` — same file on disk. Options:
  (a) they share a single source, (b) one writes back and the other
  only reads the setting. Lean (b).
- Whether to detect + inject the entire `~/.config/gcloud/` or just
  ADC. Full directory = more complete but drags state we may not
  want.

## Exit criteria

- `capsem detect` shows gcloud account + project + ADC when the user
  has gcloud authenticated.
- VM boot installs gcloud CLI and `gcloud projects list` works out of
  the box.
- Settings > Connectors > Google Cloud has credential, project, and
  account fields.
