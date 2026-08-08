# 1.3 Finalizing Sprint Plan

Status: closed.

## Purpose

Close the 1.3 branch cleanly without reintroducing old policy paths or hiding
unfinished security architecture behind UI/compatibility paint.

## Final Decision

The original parent plan was superseded by the focused
`snapshot-restore/` sprint after we found the cleanup snapshot had removed real
1.2/1.3 foundations. The final implementation and evidence are therefore
tracked in:

- `MASTER.md`
- `tracker.md`
- `snapshot-restore/MASTER.md`
- `snapshot-restore/tracker.md`

## Preserved Contracts

### Profile Contract

Capsem operates on independent profiles. A VM executes exactly one immutable
profile id.

Profile owns VM behavior:

- assets,
- VM/runtime defaults,
- enforcement rules and defaults,
- detection rules,
- MCP servers/tools/config,
- plugin config,
- availability,
- profile name, description, and icon.

Settings own only UI/application preferences.

Corp owns constraints, locks, reporting, and integrations over profiles.

### API Contract

- Profile authoring is profile-addressed.
- VM runtime/lifecycle routes live under `/vms`.
- Service-global endpoints report service/runtime/ledger state only.
- `info` means configuration/metadata.
- `status` means runtime state, counters, readiness, or progress.
- `list` means collection.
- `latest` means DB-backed ledger rows.
- `edit` means configuration mutation.
- `reload` means re-read/apply owned config files.
- HTTP and UDS expose the same route/DTO/error contract.

### Security Contract

- Security decisions run through typed `SecurityEvent` plus
  `SecurityRuleSet`/CEL.
- Policy-v2, domain-policy, and MCP decision-provider rails stay burned.
- Network and MCP own mechanics, not allow/ask/block decisions.
- Defaults are visible real rules in the same rule set.
- Plugins own audited runtime effects; rules do not secretly invoke plugins.
- Credential brokerage is opaque plugin/runtime evidence with BLAKE3
  references, not host credential injection or settings writeback.
- The ledger is forensic truth.

### UI Contract

- UI reflects backend/profile/corp/settings contracts.
- One editor writes one backing contract.
- UI does not invent backend-owned names, reasons, descriptions, rule actions,
  plugin labels, MCP labels, asset names, or credential state.
- Direct boolean fields use boolean controls; enum fields use enum controls;
  numeric fields use numeric controls with backend constraints.

## Done Criteria

- [x] Profile/settings/corp ownership is codified and tested.
- [x] Service/gateway route contract is explicit and old routes fail closed.
- [x] Old decision engines are burned.
- [x] Profile asset/admin/TUI/Linux/benchmark work lost in the cleanup snapshot
  is restored or explicitly handed off.
- [x] EROFS/LZ4HC is the 1.3 asset/rootfs contract.
- [x] Docs, skills, and changelog describe implemented behavior only.
- [x] Local smoke, VM doctor, snapshot paths, package build handoff, and
  benchmark gates are recorded.
- [x] Branch is committed and pushed.

## Accepted Handoff

Linux runtime KVM/DAX execution is not locally runnable on macOS and remains an
explicit Linux-team/CI handoff. The Linux-team scoped code and benchmark
harnesses are restored.
