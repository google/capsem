# AGY1.1.3 lazy MCP declaration

`agy_1_1_3_lazy_mcp_request.json` is a reduced first original known-synthetic
request from parent native owner39342 at source5e10e6719f3fa10966fbf596bf5654bb9ed3edec.
Original transcript BLAKE3:
`24c561058bc1842876015b3cd14b06327d1a34e3e1ee3a4bf8b5638fdeb2a91f`.
The immutable candidate is OCI762d070/root21b860c0, CLI1.1.3. Full source/root
receipt is primary ignored `.sprinty/agy-oauth-capture-cohort.json`.

Retained unchanged: model/requestType, original contents/user prompt and token,
the actual `<mcp_servers>` block, dispatcher name, parameter types/properties
and required fields. Removed unrelated system instructions, other tools,
descriptions and unrelated top-level request metadata. It is a schema/control
fixture, not byte-for-byte replay or a claim of observed successful MCP output.
All authentication was synthetic; no credential headers or user-home data
are in this fixture.

The original mock called run_command and incorrectly finalized its permission
error, which the strict native MCP ledger test caught as zero calls. Dispatch
must instead use declared call_mcp_tool with ServerName=capsem,
ToolName=local__echo and Arguments.text equal to the original proof token.
At this first capture, successful AGY functionResponse rendering was still
unknown. The original native ledger/archive test remained mandatory.

## Original successful result rendering, captured at30fc

`agy_1_1_3_mcp_result_request.json` reduces the actual third request from native
owner30457, source30fc805baaceb8e9b4026905ba1ad98f6ced77c8. Transcript BLAKE3:
`30b21b91a0715dcd4390d4ea2023dd8924da4bab3bffa0b892e2e5d5d39cb53f`.
All three contents (original prompt, model call, function response), the actual
MCP block and selected declaration shape are retained. Unrelated system/tools
metadata and descriptions are omitted as above; no credential headers exist.

The exact matched functionResponse is role=model, name=call_mcp_tool,
id=capsem_google_mcp_echo, and response.output is the three-line string:

```text
Created At: 2026-10-24T03:35:29Z
Completed At: 2026-10-24T03:35:29Z
049b8ed775cb49ad8627b8ed953dae39
```

There is no trailing newline in the actual output. Both timestamps must parse
as timezone-aware RFC3339, created must not exceed completed, and all remaining
bytes must be exactly the original nonce. No prose, error or substring fallback.
Captured wrapper role=model remains distinct from role=user typed MCP controls;
swapped/unknown roles fail. Parent independently observed exactly one physical
allowed/responded local__echo row, process agy-real, exact standard MCP request
and successful content/isError=false result. CLI finalization still failed on
the old mock400; the full original native assertion must run after this repair.
