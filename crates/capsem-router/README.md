# capsem-router

The confined host companion carries TCP bytes between published loopback ports
and connected guest VSOCK streams. The core publication broker owns destination
selection and guest connection setup. The router has no hypervisor or service
control dependency and does not resolve destinations or open outbound sockets.

## Current descriptor and process ownership

The VM owner binds the loopback listener and launches the companion with a
private Unix control socket. Only the owner sends descriptor-bearing grants.
The child reports fixed-size connection events; the parent validates connection
IDs and bounds outstanding setup work before granting a guest stream.

After runtime initialization, the companion installs Seatbelt on macOS or
seccomp on Linux before reporting readiness. It closes unrelated inherited
descriptors, receives a cleared environment, and exits when its parent dies.
It receives no virtualization entitlement. Two Tokio workers relay at most
128 connections per companion. Published mappings currently each own a child.

The parent retains the guest connection owner while the router holds a cloned
data descriptor. Setup cancellation closes pending descriptors. The relay uses
bounded asynchronous copying, including TCP half-close handling. Operational
connection failures are reported to the broker through the control lifecycle.

## Networking extension boundary

The networking sprint will move acceptance to the trusted owner and grant pairs
of connected descriptors, put connection admission through the existing
SecurityEvent pipeline, and add private VM traffic through the existing guest
net-proxy and DNS paths. Those behaviors are not provided by this rename.

Policy evaluation, audit storage, and VM control remain outside this companion.
See the [crate and privilege model](../../skills/site-architecture/references/crate-and-privilege-model.md)
for the surrounding owners. Subprocess tests exercise the executable; foundation
tests exercise OS authority denial, and Kingslanding proves published Redis
traffic in real VMs.
