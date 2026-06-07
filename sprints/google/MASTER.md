# Google Integration Sprint

Last updated: 2026-05-29

## Mission

Make Google a first-class Profile V2 integration family instead of scattering
Gmail, Drive, gcloud, Firebase, Jet Ski, Gemini, and Google AI across the
Foundation board.

This sprint is owned by the Profile Foundation sprint:

- Foundation F06 depends on Google credential brokerage decisions here.
- Foundation F10 depends on Google product integration decisions here.
- Foundation F07 consumes Google graph/dashboard/observability outputs.
- Foundation F09 consumes Firebase remote decision/alert behavior if it becomes
  a security plugin or remote-control channel.

## Scope

Google work is in scope when it touches any of:

- Gmail
- Drive
- gcloud / Application Default Credentials
- Firebase projects and service-account JSON
- Firebase Realtime Database for remote communications
- Jet Ski
- Gemini / Google AI
- Google-backed MCP tools or generated tools
- Google identity, consent, scopes, freshness, revocation, audit, status, graph,
  dashboard, and support-bundle behavior

## Execution Order

| # | Sprint | Status | Purpose |
| --- | --- | --- | --- |
| 0 | [G00 - Google Inventory And Baseline](G00-google-inventory-baseline.md) | Active | Inventory existing code, host files, env vars, profile fields, docs, and install behavior before designing new Google flows. |
| 1 | [G01 - Google Account And Credential Brokerage](G01-google-account-credential-brokerage.md) | Not Started | Normalize Google OAuth, gcloud ADC, service accounts, Gemini keys, Firebase credentials, scopes, revocation, and audit into Profile V2 credential brokerage. |
| 2 | [G02 - Gmail And Drive Integration](G02-gmail-drive-integration.md) | Not Started | Make Gmail and Drive profile-owned tool/connectors with review, scopes, evidence, diagnostics, and redaction. |
| 3 | [G03 - gcloud And Firebase Project Tooling](G03-gcloud-firebase-project-tooling.md) | Not Started | Support gcloud CLI/project context, Firebase CLI/project selection, service accounts, and project-level diagnostics. |
| 4 | [G04 - Jet Ski Integration](G04-jetski-integration.md) | Not Started | Capture Jet Ski identity, credentials, tool behavior, evidence, and UI/status requirements once the exact tool contract is confirmed. |
| 5 | [G05 - Firebase Realtime DB Remote Comms](G05-firebase-realtime-db-remote-comms.md) | Not Started | Treat Firebase Realtime Database as the remote communications path, with auth, channel model, redaction, replay, alert, and failure semantics. |
| 6 | [G06 - Gemini And Google AI Provider](G06-gemini-google-ai-provider.md) | Not Started | Make Gemini/Google AI model provider behavior profile-owned, audited, metered, and explainable in Security Events. |
| 7 | [G07 - Google Graph Dashboard And Support](G07-google-graph-dashboard-support.md) | Not Started | Add Google nodes/edges to the product graph, dashboard, status/debug, reporting, and support bundles. |

## Exit Criteria

- A single approved Google account/credential story covers Gmail, Drive,
  gcloud, Firebase, Jet Ski, Gemini, and Google AI without duplicate setup per
  surface.
- Every Google-backed capability is profile-owned and has explicit allowed,
  denied, stale, missing, revoked, and locked behavior.
- Firebase Realtime DB remote comms has a documented channel model and
  fail-closed security behavior.
- Google-backed calls produce canonical Security Events, tool/model evidence,
  metrics, audit rows, graph edges, and redacted support-bundle output.
- UI/status/dashboard can say which Google capability is configured, blocked,
  stale, or unavailable and why.

## Foundation Handoff

- F06 owns the general credential broker. This sprint gives F06 its Google
  credential families and acceptance criteria.
- F10 owns product integrations. This sprint gives F10 the concrete Google
  surfaces.
- F07 owns graph/dashboard/observability. This sprint gives F07 Google graph
  nodes, remote comms status, and dashboard states.
- F09 owns security plugins and remote decisions. This sprint gives F09 the
  Firebase Realtime DB remote comms interface if it is used for remote
  decisions or alerts.
