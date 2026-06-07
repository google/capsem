# F02 - Security Event Contract Closure

## Goal

Freeze the canonical Security Event and Resolved Security Event contracts.

## Scope

- Event family/type, source engine, redaction, trace, profile, session, and
  accounting identity.
- Resolved event steps, detection findings, links, plugin transforms, and final
  actions.
- Pack identity, schema fixtures, compatibility checks, and unknown-field
  rejection.
- Quota dimensions needed by F11.

## Acceptance Criteria

- Schema fixtures cover every event family.
- Contract tests prove roundtrip and unknown-field rejection.
- Plugin transform records and immutable-field rules are pinned.
- Docs name exactly what future code may extend.
