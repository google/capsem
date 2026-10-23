# Finite host policy and settings route cases

The existing `routes` collector owns `[benchmark.routes.policy]`. Each full
cohort uses three independent service homes per transport (quick uses one).
Every case changes real owned settings/corporate files and restores their exact
original bytes, absence and permissions. Writes are finite, never concurrent
512-request loads. The configuration fixes the builtin local tool and dummy
plugin, so no vendor, tool invocation, credential or approval fixture is needed.
The cached local tool catalogue remains explicitly unapproved.

The six operation contracts are:

- `PATCH /settings/edit`: change a real font-size setting; compare its file and
  fresh public effective value and ownership.
- `PUT /corp/edit`: install exact inline corporate TOML and provenance, then
  verify the independently read corporate file, source metadata and corp-owned
  effective setting. No URL source is used.
- `POST /corp/validate`: parse valid inline TOML and prove it installed or
  modified none of the owned files.
- `PATCH /plugins/{plugin_id}/edit`: change the dummy plugin's mode and verify
  both the owned configuration and a fresh effective-host plugin read.
- `PATCH /mcp/default/edit`: change the default permission and verify the file,
  public effective host permission and returned audit acknowledgement.
- `PATCH /mcp/servers/{server_id}/tools/{tool_id}/edit`: change an existing
  canonically created managed rule for the cached local tool; verify its exact
  target condition, persisted action and fresh effective-host permission.

Each case proves the public VM set is empty before and after its operation.
These are **host-only filesystem/effective-config** cases. They do not certify
acknowledgement by a running VM, broker enforcement in that VM, power-loss
file durability, or a physical `policy_mutation_events` row. Settings and corp
routes do not emit that ledger event at all. Plugin/MCP physical ledger proof
remains with `tests/ironbank/test_policy_mutation_routes.py` after producer drain;
real running-owner acknowledgement remains with the policy-push owner. Both
are explicit additional case debt. The collector opens no database and adds no
projection, telemetry reader, endpoint or authority.

Returned MCP audit identities are compared against independently captured file
bytes through the existing accelerated `capsem_builder.crypto` BLAKE3 owner.
This checks the existing security-audit wire contract outside request clocks;
it does not claim that the buffered ledger event has reached disk. Corporate
provenance's existing BLAKE3 format is verified through the same owner. No new
hash/checksum or hashing dependency is introduced.

Strict HTTP headers/first-body/completion receipts use the same single-attempt
transport as other route cases. `verified_host_completion_ms` includes the
request plus independent file/public-readback checks, not physical VM scanout
or a pure service execution clock. Exact restore and `corp/reload` with zero
running targets occur after that clock. Process-tree resource windows include
baseline verification, measured action and restore/readback; their wall/CPU/RSS
receipts must not be described as isolated endpoint CPU.

`ServiceInstance(isolated_corp_config=True)` sets the child-only corporate input
to its own home, overriding ambient/system corp configuration without editing
the parent's environment. Fixture setup uses the canonical MCP permission edit
once outside clocks to create its managed rule. No desired-state echo or
already-cached no-op may stand in for a successful measured change. Parsed
settings must preserve every unrelated policy field; only the selected setting
and its validated timestamp may change. Missing,
refused, partially observed, un-restorable or wrong-count cases fail the entire
collector. All derived statistics, raw sample storage and fitness admission
remain the existing Rust owners.

Source tests replace service/HTTP launches and cannot certify these operation
latencies. Exact-source native execution, machine fitness and repeated real
measurements remain required before a performance claim.

Three finite samples are raw observations, not a statistically established p99
or capacity result. Repeated fit-cohort execution is still required.
