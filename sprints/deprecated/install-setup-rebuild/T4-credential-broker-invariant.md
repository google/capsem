# T4: Brokered Substitution Plugin Invariant

## Purpose

The brokered substitution plugin is part of the security spine, not setup,
onboarding, or an optional convenience feature. Its job is to make sensitive raw
material short-lived. Credentials are the first material class: Capsem observes
credential material, saves it through one trusted credential-store path, writes
only a stable reference to settings, and replaces the raw value with that
reference before the rest of the system can log, display, persist, or evaluate
it.

## Contract

```text
VM/workspace/OAuth credential observation
  -> brokered substitution pre-plugin
  -> classify provider/source/confidence
  -> save raw credential to broker credential store
  -> write stable BLAKE3 credential reference to user settings
  -> return stable BLAKE3 credential reference
  -> security events, session.db, logs, CEL, enforcement, UI, and DB see metadata only
```

The broker is on by default. Autosave to the broker credential store is on by
default. User settings store only stable references. Ask-before-save,
autosave-off, and broker-disable settings are later product controls and must
not fork the first implementation.

Substitution logging is also part of this plugin. Every replacement emits a
redacted substitution log record with material class, source, event type,
algorithm, reference/fingerprint, outcome, and session/profile context.

Credential substitution format:

```text
credential:blake3:<hex>
```

The digest input is domain-separated, for example:

```text
capsem.credential.v1 || provider || raw_credential
```

The exact byte framing should be implemented once in the broker crate/module
and tested; callers must not build substitution strings themselves.

## Invariants

- Raw credentials must not reach ordinary security events, logs, CEL previews,
  database previews, or UI/API responses.
- Every observed credential is brokered through the same write path.
- Every substitution is logged by the same plugin path, not by protocol-specific
  substitute loggers.
- The write sink for v1 raw secrets is the broker credential store. On macOS
  production builds use Keychain; tests use an explicit file-backed store. User
  settings store only stable broker references.
- Downstream systems receive and log a stable BLAKE3 reference plus metadata:
  provider, source, fingerprint, confidence, session/profile context, and
  outcome.
- Security events and `session.db` rows carry the BLAKE3 reference/fingerprint
  as top-level shared security-event identity fields.
- The contract is protocol agnostic: HTTP authorization headers, GitHub tokens,
  OAuth/token exchanges, `.env` files, model payloads, MCP arguments, file
  content, and process/environment observations must all flow through the same
  brokered credential identity shape.
- The broker logs that it acted, including the BLAKE3 reference/fingerprint,
  not what the secret was.
- Substitution logs are queryable evidence that a raw value was replaced, but
  they never carry the raw value.
- Host credential scraping during install/setup is out of scope and should be
  removed or reduced to diagnostics.

## Initial Observations

- `.env` style variables inside VM/workspace:
  `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`,
  `ANTHROPIC_API_KEY`.
- Visible OAuth/token exchange material that crosses the VM security path.
- HTTP authorization headers and bearer tokens.
- GitHub tokens such as `ghp_*`/`github_pat_*` when observed in the VM/security
  path.
- Future credential-bearing protocols and file/process/model/MCP payloads.

## Implemented Backend Shape

- `capsem-logger` owns canonical `credential:blake3:<hex>` generation and
  validates `credential_ref` / `substitution_ref` in new SQLite schemas.
- `capsem-core::credential_broker` owns observation, provider classification,
  user-settings writes, substitution log emission, and byte-preview redaction.
- MITM header formatting substitutes recognized credentials before telemetry;
  unknown sensitive headers remain short-hashed.
- `TelemetryHook` detects request/response JSON body tokens before building
  `NetEvent` / `ModelCall`, redacts previews, writes substitution rows, and
  carries the shared `credential_ref`.
- `FsMonitor` brokers small `.env` / `.env.*` files observed in the workspace
  path and records the same shared reference on `fs_events`.
- Typed logger readers expose `credential_ref` for new DBs and gracefully return
  `None` for old read-only session fixtures that predate the column.

## Wizard Simplification

The old AI setup wizard should disappear as a credential collection path.
Provider configuration happens inside the VM where the user already performs
the real tool login/configuration. Capsem then brokers the observed credential
into the credential store, writes a reference to user settings, and exposes only
broker status/reference metadata.

The finalization UI may include a short architecture/session page explaining:

- credentials are brokered by default;
- raw secrets are stored in the broker credential store and replaced with
  BLAKE3 references in settings/events;
- logs and security events never contain raw secrets;
- future settings may let users choose ask-before-save or disable autosave.

## Tests Required

- Fake `.env` key becomes a user-settings entry and a credential reference.
- Fake OAuth/token exchange becomes a user-settings entry and a credential
  reference.
- The substitution value is deterministic BLAKE3 for the same credential and
  changes when the credential changes.
- The substitution value includes the broker domain/provider framing, not a
  generic BLAKE3 hash of the bare credential.
- Raw secret strings are absent from security events, logs, DB previews, and UI
  API responses.
- Broker logs and security events include the BLAKE3 reference/fingerprint so
  credential use can be audited and correlated without raw secret material.
- Substitution log records include material class, source, event type,
  algorithm, reference/fingerprint, outcome, and context.
- `session.db` security-event rows include the BLAKE3 reference/fingerprint as
  shared event fields. Typed protocol rows add context only and must not own a
  second credential identity.
- HTTP/GitHub fixtures prove the broker path is protocol agnostic and not an
  AI-only lane.
- Two callers cannot write credentials through separate settings paths.
- CEL/security rules can match credential metadata/reference without seeing the
  raw value.
