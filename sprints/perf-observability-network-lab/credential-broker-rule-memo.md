# Credential Broker Rule Memo

Status: handoff memo for main-agent reconciliation.

Upstream reviewed: `Infisical/agent-vault` at
`234dbf0d27d4749b35690c91713fd2789c810cd7`.

Source of truth for the upstream provider catalog:
`internal/catalog/catalog.go`.

Relevant upstream mechanics:
- `internal/broker/broker.go`: `SupportedAuthTypes`,
  `SubstitutionSurfaces`, auth validation, and auth rendering.
- `internal/brokercore/substitution.go`: path/query/header/body/websocket
  substitution behavior.
- `internal/brokercore/brokercore.go`: broker request/response sanitation.

## Rule Shape

Broker authoring is one block. The block contains the credential action, the
credential rendering type, and the CEL rule that decides when it applies.

```toml
[ai.example.credentials.EXAMPLE_API_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.example.com" || credential.name == "EXAMPLE_API_KEY"'
priority = 0
```

Do not split this into a credential block plus a separate match rule.

Rules:
- Use `rule = '<CEL expression>'`.
- Do not author `on`; infer event families from first-party CEL roots.
- Do not use `decision`.
- Do not use `detect`.
- Do not add `storage`, `key`, or `aliases`.
- Action rules use `priority = 0`.
- Secrets remain Keychain-backed. Logs and session DB store only BLAKE3
  substitution references.

Capsem target credential types:
- `api-key`: named header plus optional prefix.
- `basic`: `Authorization: Basic base64(username:password)`.
- `custom`: validated header/body templates with credential placeholders.
- `substitution`: validated placeholder replacement in path/query/header/body
  and websocket payloads.
- `oauth2`: access/refresh-token materialization with security-event logging.

Agent Vault source note: their service auth types are `bearer`, `basic`,
`api-key`, `custom`, and `passthrough`; `bearer` maps to Capsem `api-key` with
`Authorization: Bearer `. `passthrough` is not a broker credential action.

## Exhaustive Agent Vault Catalog Conversion

These are all 22 templates from `internal/catalog/catalog.go`, converted into
Capsem one-block credential rules.

```toml
[ai.anthropic.credentials.ANTHROPIC_API_KEY]
action = "credential"
type = "api-key"
header = "x-api-key"
rule = 'http.host == "api.anthropic.com" || model.provider == "anthropic" || credential.name == "ANTHROPIC_API_KEY"'
priority = 0

[ai.aws-s3.credentials.AWS_SECRET_ACCESS_KEY]
action = "credential"
type = "custom"
headers = { Authorization = "{{ AWS_SECRET_ACCESS_KEY }}" }
rule = 'http.host == "s3.amazonaws.com" || http.host.matches("(^|.*\\.)s3\\.amazonaws\\.com$") || credential.name == "AWS_SECRET_ACCESS_KEY"'
priority = 0

[ai.cloudflare.credentials.CLOUDFLARE_API_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.cloudflare.com" || credential.name == "CLOUDFLARE_API_TOKEN"'
priority = 0

[ai.datadog.credentials.DATADOG_API_KEY]
action = "credential"
type = "api-key"
header = "DD-API-KEY"
rule = 'http.host == "api.datadoghq.com" || credential.name == "DATADOG_API_KEY"'
priority = 0

[ai.github.credentials.GITHUB_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.github.com" || credential.name == "GITHUB_TOKEN"'
priority = 0

[ai.jira.credentials.JIRA_API_TOKEN]
action = "credential"
type = "basic"
username = "JIRA_EMAIL"
password = "JIRA_API_TOKEN"
rule = 'http.host.matches("(^|.*\\.)atlassian\\.net$") || credential.name == "JIRA_EMAIL" || credential.name == "JIRA_API_TOKEN"'
priority = 0

[ai.linear.credentials.LINEAR_API_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.linear.app" || credential.name == "LINEAR_API_KEY"'
priority = 0

[ai.notion.credentials.NOTION_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.notion.com" || credential.name == "NOTION_TOKEN"'
priority = 0

[ai.npm.credentials.NPM_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "registry.npmjs.org" || credential.name == "NPM_TOKEN"'
priority = 0

[ai.npmgh.credentials.NPM_GH_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "npm.pkg.github.com" || credential.name == "NPM_GH_TOKEN"'
priority = 0

[ai.openai.credentials.OPENAI_API_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.openai.com" || model.provider == "openai" || credential.name == "OPENAI_API_KEY"'
priority = 0

[ai.pagerduty.credentials.PAGERDUTY_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.pagerduty.com" || credential.name == "PAGERDUTY_TOKEN"'
priority = 0

[ai.postmark.credentials.POSTMARK_SERVER_TOKEN]
action = "credential"
type = "api-key"
header = "X-Postmark-Server-Token"
rule = 'http.host == "api.postmarkapp.com" || credential.name == "POSTMARK_SERVER_TOKEN"'
priority = 0

[ai.resend.credentials.RESEND_API_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.resend.com" || credential.name == "RESEND_API_KEY"'
priority = 0

[ai.sendgrid.credentials.SENDGRID_API_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.sendgrid.com" || credential.name == "SENDGRID_API_KEY"'
priority = 0

[ai.sentry.credentials.SENTRY_AUTH_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "sentry.io" || credential.name == "SENTRY_AUTH_TOKEN"'
priority = 0

[ai.shopify.credentials.SHOPIFY_ACCESS_TOKEN]
action = "credential"
type = "api-key"
header = "X-Shopify-Access-Token"
rule = 'http.host.matches("(^|.*\\.)myshopify\\.com$") || credential.name == "SHOPIFY_ACCESS_TOKEN"'
priority = 0

[ai.slack.credentials.SLACK_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "slack.com" || credential.name == "SLACK_TOKEN"'
priority = 0

[ai.stripe.credentials.STRIPE_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.stripe.com" || credential.name == "STRIPE_KEY"'
priority = 0

[ai.supabase.credentials.SUPABASE_KEY]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host.matches("(^|.*\\.)supabase\\.co$") || credential.name == "SUPABASE_KEY"'
priority = 0

[ai.twilio.credentials.TWILIO_AUTH_TOKEN]
action = "credential"
type = "basic"
username = "TWILIO_ACCOUNT_SID"
password = "TWILIO_AUTH_TOKEN"
rule = 'http.host == "api.twilio.com" || credential.name == "TWILIO_ACCOUNT_SID" || credential.name == "TWILIO_AUTH_TOKEN"'
priority = 0

[ai.vercel.credentials.VERCEL_TOKEN]
action = "credential"
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host == "api.vercel.com" || credential.name == "VERCEL_TOKEN"'
priority = 0
```

## Non-Catalog Broker Capabilities To Keep

Agent Vault has substitution mechanics even though the catalog above does not
list a substitution credential. Capsem should keep it as a first-class
credential action:

```toml
[ai.twilio.credentials.TWILIO_ACCOUNT_SID_PATH]
action = "credential"
type = "substitution"
placeholder = "{{ TWILIO_ACCOUNT_SID }}"
value = "TWILIO_ACCOUNT_SID"
rule = 'http.host == "api.twilio.com" || credential.name == "TWILIO_ACCOUNT_SID"'
priority = 0
```

OAuth2 is also required for Capsem even though Agent Vault's service catalog
does not model it as a service auth type:

```toml
[ai.google.credentials.GOOGLE_OAUTH]
action = "credential"
type = "oauth2"
authorization_url = "https://accounts.google.com/o/oauth2/v2/auth"
token_url = "https://oauth2.googleapis.com/token"
client_id = "GOOGLE_OAUTH_CLIENT_ID"
client_secret = "GOOGLE_OAUTH_CLIENT_SECRET"
scopes = ["https://www.googleapis.com/auth/cloud-platform"]
header = "Authorization"
prefix = "Bearer "
rule = 'http.host.matches("(^|.*\\.)googleapis\\.com$") || model.provider == "google" || credential.name == "GOOGLE_OAUTH"'
priority = 0
```

## Tests Required

- TOML parses every block above.
- CEL compiler accepts every `rule`.
- Event-family inference compiles each rule to the right callback set.
- Every credential action has `priority = 0`.
- `detect`, `decision`, `on`, `storage`, `key`, and `aliases` are rejected in
  broker credential blocks.
- Rendering tests cover `api-key`, bearer-as-api-key, `basic`, `custom`,
  `substitution`, and `oauth2`.
- Logging tests prove only BLAKE3 references reach security events and
  `session.db`.
