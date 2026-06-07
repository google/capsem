# G00 - Google Inventory And Baseline

## Goal

Find the real current Google state before designing new flows.

## Scope

- Existing code for Gemini, Google AI, gcloud ADC, OAuth forwarding, setup, and
  guest injection.
- Host paths such as `~/.config/gcloud/application_default_credentials.json`,
  service-account JSON, Gemini settings, Firebase config, and any Jet Ski
  credential/config files.
- Current profile fields, service settings, docs, setup wizard behavior, and
  support-bundle redaction.

## Acceptance Criteria

- Every discovered Google credential/source path is recorded.
- Existing tests and gaps are named.
- Unknowns, especially Jet Ski contract details, are visible in the tracker.
