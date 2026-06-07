# G07 - Google Graph Dashboard And Support

## Goal

Represent Google integration state in the product graph, dashboard, reporting,
and support bundles.

## Scope

- Graph nodes and edges for account, credential, project, profile, VM, session,
  tool, provider, Firebase channel, remote message, alert, and Security Event.
- Dashboard state for Gmail, Drive, gcloud, Firebase, Jet Ski, Gemini, and
  Google AI.
- Reporting and support-bundle redaction.

## Acceptance Criteria

- Dashboard does not recompute Google truth from ad hoc sources.
- Graph links Google activity to canonical event ids where security evidence is
  involved.
- Support bundles show enough status to debug without leaking credentials.
