---
title: Errors and recovery
description: How do SDK errors, timeouts, cancellation, and retries behave?
---

This page explains SDK errors, deadlines, cancellation, and connection lifetime
so you can handle failures without unintentionally repeating an operation.
A request failure can leave its
result uncertain: the service may have accepted a change before the response
was lost. SDKs do not automatically replay mutations.

## Errors

| Language | Failure | Information retained |
| --- | --- | --- |
| Python | `HttpError` | HTTP `status`, response `body`, and recognized `credential_code`. |
| Python | Pydantic `ValidationError` | Invalid typed input or response. |
| Python | `LookupError` | A VM name matched zero or multiple VMs. |
| TypeScript | `HttpError` | HTTP `status`, response `body`, and recognized `credentialCode`. |
| TypeScript | `NetworkError` | Transport failure; no confirmed HTTP result. |
| TypeScript | Zod validation error | Invalid typed input or response. |
| Rust | `Error::Http` | HTTP status and response bytes; `credential_code()` recognizes broker refusals. |
| Rust | `Error::Json` / `Error::InvalidResponse` | Invalid response encoding or contract. |
| Rust | `Error::VmLookup` | The requested name did not resolve to exactly one VM. |

See each [language reference](/sdk/) for its exact error types. Credential
refusal codes are defined in the [credentials reference](/reference/credentials/).

## Deadlines and cancellation

| Setting | Python | TypeScript | Rust |
| --- | --- | --- | --- |
| Client HTTP deadline | `timeout` in seconds | `timeoutMs` | `with_timeout(Duration)` |
| Command deadline | `timeout_secs` | `timeout_secs` | `timeout_secs` argument |
| Cancel a request | Cancel its coroutine | Pass an `AbortSignal` | Drop its future |

The default HTTP deadline is 30 seconds. Create, start, and resume allow at
least 230 seconds for workload readiness. Exec and run allow at least the
command deadline plus 120 seconds; without a command deadline they use the
service's one-hour command ceiling plus that budget. A larger client deadline
is retained unless a per-call override is supplied.

Cancelling an HTTP request does not undo an accepted operation. Inspect the
VM's state before deciding whether another request is needed. After an uncertain
create, find the VM before creating another with the same intended purpose.

## Connections and VM lifetime

VMs returned by create, selection, and fork share their client's connection.
Closing a Python or TypeScript client releases its HTTP resources; dropping
Rust handles releases their shared pool when the last handle is gone.
Closing a handle does not stop or delete its VM.

Use managed scopes for temporary jobs that need service-owned cleanup.
They create once, renew a finite ownership lease, and await cleanup of the
original VM generation. Cancellation does not skip the cleanup barrier. If
the client crashes, renewal stops and the service expires the lease. A failed
cleanup is an error; retry that close explicitly.

## Command output

Execution results retain the exit code, separate stdout and stderr, their
encoding, and the truncation flag. UTF-8 and base64 wire values both represent
bytes. Python exposes `stdout_bytes` and `stderr_bytes`; TypeScript exposes
`decodeExecOutput`; Rust's `ExecOutput::decode()` returns the exact bytes.

An HTTP success does not mean a command exited successfully. Check
`exit_code` before using its output as a successful result.

## Service restart

Restart returns an HTTP 202 acknowledgement. It requires an idle service
managed by launchd or systemd: active or starting VMs return 409; an unmanaged
service returns 503. Acceptance does not confirm that reconnection is complete.

The gateway token rotates on restart. Create a fresh authenticated connection;
do not replay restart or retry it with stale credentials.
