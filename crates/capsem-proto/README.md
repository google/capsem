# capsem-proto

Shared protocol types used across Capsem crates. Two protocol families:

- **Host-to-guest / guest-to-host** -- messages on the vsock control plane
  (`HostToGuest`, `GuestToHost`).
- **Service-to-process / process-to-service** -- IPC between `capsem-service`
  (the daemon) and the per-VM `capsem-process` supervisor (`ServiceToProcess`,
  `ProcessToService`).

No business logic lives here. See <https://capsem.org/architecture/service-architecture/>
for how the protocols fit into the system.
