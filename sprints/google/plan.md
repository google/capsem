# Google Integration Plan

## What We Are Building

A dedicated Google integration sprint under `sprints/google/` that splits
Google out of the Foundation board and gives each painful surface its own lane:
Gmail, Drive, gcloud, Firebase, Jet Ski, Firebase Realtime DB remote comms,
Gemini, and Google AI.

## Key Decisions

- Google gets its own sprint folder because it has multiple credential shapes,
  APIs, tools, and runtime behaviors.
- Firebase Realtime Database remote comms is its own sub-sprint, not a detail
  inside generic Firebase support.
- Jet Ski is kept as a named sub-sprint while the exact tool contract is
  clarified.
- Foundation F06/F10/F07/F09 depend on this sprint rather than carrying Google
  details inline.

## Files Modified

- `sprints/google/MASTER.md`
- `sprints/google/tracker.md`
- `sprints/google/G*.md`
- `sprints/profile-foundation/MASTER.md`
- `sprints/profile-foundation/F06-credential-brokerage-foundation.md`
- `sprints/profile-foundation/F10-product-integration-foundation.md`
- `sprints/README.md`
- `CHANGELOG.md`

## Done

- Google is listed as an active sprint board.
- Foundation points to `sprints/google/` for Google-specific detail.
- Firebase Realtime DB remote comms and Jet Ski are named sub-sprints.
