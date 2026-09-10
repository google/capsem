# capsem-router

The confined host companion carries TCP bytes between published loopback ports
and connected guest VSOCK streams. The core publication broker owns destination
selection and guest connection setup. The router has no hypervisor or service
control dependency and does not resolve destinations or open outbound sockets.

## Current descriptor and process ownership

The VM owner binds and accepts the loopback listener, connects the guest VSOCK
stream, and grants the two connected descriptors over a private Unix socket.
The companion receives no listener. Linux also denies accept syscalls; macOS
preserves existing FD authority, so closing inherited FDs and rejecting listener
grants are essential. Ten-byte,
versioned headers carry grants, acknowledgements, closes, and aborts; only grants
carry FDs. Close reports append 17 bytes for a typed reason and two delivered-byte
counts. Cooperative cancellation preserves those counts during active transfer
and FIN drain. Oversized records, excess FDs, reused IDs, and unexpected events fail
closed. Kernel-sized ancillary storage prevents truncated FD ownership on Darwin.

After runtime initialization, the companion installs Seatbelt on macOS or
seccomp on Linux before reporting readiness. It closes unrelated inherited
descriptors, receives a cleared environment, and exits when its parent dies.
It receives no virtualization entitlement. Two Tokio workers relay at most
64 expose and 64 private connections per companion, without borrowing between
classes. Published mappings share one VM-owned child; private routing is not
yet connected to the VM broker. Core serializes grants across all mappings and
dispatches bounded acknowledgements to the broker retaining each endpoint pair.
Expose admission is shared across every listener in the VM: 64 queued/active
connections, eight guest setups, and 32 setup requests per second with a burst
of 16. Setup waits count against the eight-second deadline. Cancelling a queued
request returns its permits and preserves rate credit; pacing owns no refill task.

The parent retains shutdown handles for both endpoints until the child reports
closure. Guest setup has an eight-second deadline and pair acknowledgement a
two-second deadline. Control failure shuts down both sides even if the child
holds duplicate FDs. Cancellation closes partial records and received FDs.
The sender retains its original descriptors through acknowledgement: on Darwin,
a socket referenced only by queued descriptor messages can be garbage collected.
Data sockets request fixed 64 KiB kernel queues on both ends, with Linux VSOCK's
separate credit limits fixed to the same value. Linux TCP reports up to twice
that request for kernel bookkeeping. These limits are set before handoff and
applied again by the child; setup refuses an unsupported buffer configuration.
The relay uses 16 KiB per direction and preserves TCP half-close. Each successful
write renews a 60-second stall deadline; no deadline applies to quiet reads. After
one direction drains and sends FIN, the reverse direction has 60 seconds to
finish. Host and guest reuse the same bounded copier. Its tasks and
control reader belong to JoinSets and are cancelled and joined on control failure.
Publication removal cancels its broker and aborts its flows; late acknowledgements
are discarded until Closed without interrupting other publications. VM shutdown
joins the brokers, guest handshake readers, and child monitor before log draining.

The guest control connection owns its own bridge JoinSet. Disconnect, shutdown,
or snapshot preparation cancels streams and joins setup before returning. At most
eight disposable namespace threads set up connections; reusable Tokio workers
never change namespace. VSOCK connect, container TCP connect, and the guest
handshake share a three-second setup deadline. The PID file is opened through
the existing containment helper and bounded to a small regular-file read, so a
FIFO or symlink cannot trap the setup worker. Queued and active flows share the
guest's 64 ingress slots. The guest runtime is drained before it is dropped.

## Networking extension boundary

Subsequent networking work will put connection admission through the existing
SecurityEvent pipeline, extend admission to private setup requests,
and carry private VM traffic through the existing guest net-proxy and DNS paths.
Those behaviors are not provided by the descriptor handoff alone.

Policy evaluation, audit storage, and VM control remain outside this companion.
See the [crate and privilege model](../../skills/site-architecture/references/crate-and-privilege-model.md)
for the surrounding owners. Subprocess tests exercise the executable; foundation
tests exercise OS authority denial, and Kingslanding proves published Redis
traffic in real VMs.
