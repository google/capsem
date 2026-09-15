# capsem-router

The confined host companion runs in one of two modes, each its own process:

- **Expose** (default): one per VM, carrying TCP bytes between published
  loopback ports and connected guest VSOCK streams. The core publication broker
  owns destination selection and guest connection setup.
- **`--network`**: one per private network, that network's L2 switch. The
  service plugs and unplugs cables -- one guest tap's frame stream per VM
  membership -- and the switch forwards frames between them (see
  [Network switch](#network-switch)).

The router has no hypervisor or service control dependency, does not resolve
destinations, and never opens a socket of its own.

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
64 expose connections per companion. Published mappings share one VM-owned
child. Core serializes grants across all mappings and dispatches bounded
acknowledgements to the broker retaining each endpoint pair.
Expose admission is shared across every listener in the VM, by default: 64 queued/active
connections, eight guest setups, and 32 setup requests per second with a burst
of 16. Setup waits count against the eight-second deadline. Cancelling a queued
request returns its permits and preserves rate credit; pacing owns no refill task.

The active profile can lower connection and setup ceilings and tune setup pacing:

```toml
[network.router.expose]
connections = 32
setups = 4
rate_per_second = 16
burst = 8
```

Omitted fields retain the defaults above. Connections validate in 1–64, setups
in 1–8, setup rate in 1–1024 per second, and burst in 1–64. Zero never means
unlimited. The VM loads these budgets before restoring published listeners and
passes the connection ceiling to its confined child. Budgets apply for the VM
process lifetime.

The parent retains shutdown handles for both endpoints until the child reports
closure. Guest data handshakes include a fresh owner generation as well as the
request ID. A stale generation is rejected before pending work is consumed,
so a restarted VM cannot attach an old stream to a reused numeric request ID.
Guest setup has an eight-second deadline and pair acknowledgement a
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
The owner also sends bounded `AbortPorts` batches over that same control
connection. Each entry names the generation and request ID; it cancels only that
flow, including queued setup. Cleanup joins host setup before sending the batch,
so a late connect cannot overtake its abort. Invalid child closes retain the
guest cancellation identity until cleanup. Host control I/O is asynchronous,
with five-second write and started-frame deadlines and an owned reader.
TCP descriptors are armed for abortive close before setup/handoff and again at
child adoption, covering SIGKILL without relying on destructors. Normal Complete
clears that setting; other exits avoid an early FIN. The trusted owner immediately revokes its TCP endpoint using
linger-zero plus `disconnectx` on macOS or `connect(AF_UNSPEC)` on Linux; this
also revokes a malicious child's retained copies. The child retains no connect
authority. Guest cancellation resets the container TCP endpoint. Normal Complete
still drains both directions. Guest `PortClosed` reports keep their VSOCK stream
open after abnormal termination until `PortCloseAck`: the host revokes TCP before
acknowledging, so stream EOF cannot race an unintended FIN through the copier.
The control actor accepts reports only for the owning generation and flow lease,
but acknowledges retired duplicates to release guest credits. Child cancellation
retains its observer for final byte counts; a missing close ACK terminates the child.
Guest counters are diagnostic and do not become authoritative host audit counts.

The existing control writer reserves one of 64 network report credits before
setup. Active streams, terminal reports, queued copies and reconnect snapshots
retain that credit until both queue drain and host acknowledgment. Network messages
cannot enter the legacy queue without a credit. A five-second missing guest close
ACK fails its control lease; the writer checks this between bounded writes. Control
disconnect revokes all live TCP endpoints while leaving declared listeners available.

## Network switch

`capsem-router --network` is one network's switch and nothing else: no uplink,
no listener, no outbound socket. Its only inputs are `Plug` grants (a port id
and one connected cable descriptor), `Unplug` grants (a port id), and the
cables' frames; its only output is events to the service -- ready, accepted,
refused, and a counter report when a port closes. A port id carries the
membership's generation and address; the MAC is a function of the address, so
nothing is learned from traffic.

- A frame to a port's MAC goes to that port; a broadcast or multicast frame
  floods every other port (ARP works unchanged), capped at 1024 floods per
  second per port; an unknown destination is dropped, never flooded.
- Port security: a frame must carry its port's MAC, and an IPv4 packet or ARP
  message its port's address. Other ethertypes cross unchecked.
- A plug with a generation no newer than the address's last is refused, so a
  stale grant can never replace a current cable. At most `--port-limit` (64)
  ports are live, closing ones included.
- A port is one job owning both halves of its cable: the reader never waits on
  another port's queue (64 frames each; a full queue drops and counts), the
  writer batches vectored writes. Unplugging cancels the job, closing the
  descriptor -- even under a writer blocked on a member that stopped reading --
  before the port is reported closed and its slot reused.

The switch is a network, not a security boundary: every member plugged into a
switch can reach every other. Membership is the service's decision; policy
evaluation, audit storage, and VM control remain outside this companion. See
the [crate and privilege model](../../skills/site-architecture/references/crate-and-privilege-model.md)
for the surrounding owners. Subprocess tests exercise the executable; foundation
tests exercise OS authority denial, and Kingslanding proves published Redis
traffic and member traffic over the switch in real VMs.
