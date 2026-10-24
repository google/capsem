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
Actual successful AGY functionResponse rendering has not yet been captured.
Source typed-result controls are explicitly provisional: unknown/error output
fails, and the original native ledger/archive test remains mandatory.
