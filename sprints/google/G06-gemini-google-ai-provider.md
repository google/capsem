# G06 - Gemini And Google AI Provider

## Goal

Make Gemini and Google AI provider behavior profile-owned, audited, metered,
and explainable.

## Scope

- Gemini CLI and Google AI provider settings.
- API key, OAuth, and ADC credential paths where applicable.
- Canonical model/tool evidence.
- Token/cost/provider metrics.
- Profile capability gates, diagnostics, and support-bundle redaction.

## Acceptance Criteria

- Gemini/Google AI calls produce canonical model and tool evidence.
- Provider credentials are released only through profile/session policy.
- UI/status explains configured, missing, stale, denied, and blocked states.
