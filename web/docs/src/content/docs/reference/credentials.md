---
title: Credentials reference
description: How are credentials stored, granted to VMs, substituted, and revoked?
---

The service owns credential material. A connection, a credential reference,
and a VM grant are separate resources; saving a credential or completing
sign-in does not by itself authorize every VM to use it.

## Resources

| Resource | Meaning | Public SDK access |
| --- | --- | --- |
| Injected credential | Explicit host-supplied material; response contains an opaque reference, never the secret. | `credentials.inject` / Rust `credentials().inject`. |
| Connection | Provider authorization and supported capabilities. | `connections`: list, authorize, inspect, cancel, disconnect. |
| VM grant | Approval for selected connection capabilities in one VM owner generation. | `credential_grants` (Python/Rust) or `credentialGrants` (TypeScript). |
| Runtime credential | Material prepared for an admitted destination and operation. | Broker-owned; callers do not receive the durable store. |

## Injection

| Field | Values / requirement |
| --- | --- |
| `provider` | `anthropic`, `google`, `openai`, `github`, `mcp`. |
| `value` | Nonempty secret string; write-only input. |
| `storage` | `file` (default) or `memory`. Memory lasts for the current service lifetime. |
| Response | `credential_ref` and `storage`. |

Injection does not start browser consent or refresh a supplied OAuth token.
Use the returned reference in the workload configuration; do not print or
write the original secret into the VM's files.

## Provider authorization

Authorization is explicit. The caller retains the request ID and exact payload,
opens the returned authorization URL, and polls the transaction.
Cancellation is a separate operation. Inspect the provider catalog's enabled
profiles and capabilities before offering sign-in; unsupported enrollment
cannot be inferred from a provider's name.

Consent creates a connection. VM access requires a separate grant.

## VM grants

| Field | Contract |
| --- | --- |
| `connection_id` | Existing authorized connection. |
| `capability_ids` | Explicit capability set for this grant. |
| `expected_owner_generation` | Current generation returned by the VM's grant inventory. Stale ownership is refused. |
| `request_id` | UUID identifying this explicit grant request. |
| `expires_at` | Optional expiry timestamp. |

SDKs validate returned VM, generation, connection, and capability identities.
They do not retry grant mutations or polling implicitly.

Credential issuance checks current policy, the runtime's acknowledged policy,
and live ownership under the same admission boundary. Destination, method,
query, and port constraints still apply. A stored reference cannot override
those checks.

## Substitution and records

The security engine identifies credential references before rule evaluation.
After admission, the broker supplies credentials for the assigned upstream.
Logging prepares a separate sanitized event; restoring the real credential for
the upstream does not restore it in the stored body.

Redaction applies to recognized material. It is not a guarantee that arbitrary
user content contains no secrets. See [Security engine](/architecture/security-engine/).

## Revocation and errors

Revoke a VM grant to fence that authority; disconnect a connection to fence
local grants and begin provider retirement. Poll connection state explicitly
to confirm provider retirement. Stopping, suspending, or retiring a runtime
fences its grants; resuming binds valid authority to the new runtime.

HTTP errors preserve a known `CredentialErrorCode` when present. Unknown
codes retain their original HTTP envelope. The complete enum and resource
fields are generated in the [SDK references](/sdk/python/).
See [Errors and recovery](/sdk/errors/) for cancellation and uncertain results.
