# Guest RAM and VirtioFS performance

The October 2026 OCI boot investigation separated full image verification
into read wait and hashing. Two unchanged 352-MB image scans waited
1.53/1.57 seconds for reads and spent only 76/76 ms hashing. Optimizing the
host core and reducing FUSE request counts did not remove the delay.

The host's transparent huge-page policy was `madvise`. Capsem allocated
ordinary anonymous RAM without a huge-page hint or guaranteed 2-MiB
alignment. KVM and guest cold file I/O paid the cost of small backing pages.
An aligned lazy mapping with per-mapping `MADV_HUGEPAGE` reduced read wait
to 148/147 ms and kernel startup from 1.13 seconds to 270/280 ms. Cached
create-to-first-completed-exec was 2.18/2.05 seconds at 12 GiB and four CPUs.
These are Linux diagnostic measurements, not installed-package or Apple
qualification. Full byte/size verification remained enabled.

## Preserve the fix

- `capsem_foundation::unix::guest_memory::GuestRam` owns anonymous mapping,
  alignment, advice and unmapping. Use its pointer; do not add an application
  `mmap`, `madvise`, `libc`, `nix`, or private Unix helper.
- Align the host RAM base to the host PMD size, at least 2 MiB. Foundation
  reads the kernel's `hpage_pmd_size`; 64-KiB-page ARM hosts can require
  32 MiB instead of the x86 2 MiB. Preserve that alignment in every KVM
  memory-slot offset, including the x86 PCI-hole split. Retain the exact
  advertised RAM size and complete guest-span bounds checks.
- Alignment and `MADV_HUGEPAGE` are both needed. Keep the allocation lazy;
  ordinary pages remain a valid fallback when advice is unavailable. Do not
  change machine-wide THP settings or eagerly populate the whole VM.
- Run the real allocation regression in `hypervisor/kvm/memory/tests.rs` and
  the owner tests in `capsem-foundation/src/unix/guest_memory/tests.rs` when
  changing mappings or memory-slot layout. Resume uses the same allocation
  owner; keep checkpoint memory and open VirtioFS handles covered.

## Diagnose the next slowdown

Sparse checkpoint restoration must preserve sparse RAM. A 12-GiB native VM
took 21.5 seconds to resume because the reader copied every logical byte,
including file holes, into newly allocated physical RAM. Read allocated
extents with the foundation owner's descriptor-based extent and positioned
I/O APIs. Zero inactive RAM with the mapping owner's discard operation first;
a hole must never retain bytes from a previous state. Preserve exact logical
length checks, bounded metadata, short-read errors and the existing file
format. The checkpoint regression checks actual resident RAM and held file
and directory descriptors must survive a real suspend/resume.

The restore trace exposed another independent delay: memory restoration
finished, then guest sockets took about eight seconds to reconnect. Linux
vhost-vsock owns only RX/TX; the third virtio queue belongs to userspace.
Deliver `VIRTIO_VSOCK_EVENT_TRANSPORT_RESET` through its standing guest event
buffer after replacing the host, publish the used entry, and signal its call
eventfd through foundation. Preserve a pending reset if no buffer is ready;
do not skip standing buffers with the completed-FUSE restore constructor.
With sparse restoration and reset delivery, two native 12-GiB/four-CPU
held-file-and-directory-FD regressions resumed in 1.04/1.04 seconds.

Detect sparse checkpoint occupancy in 4-KiB pages and coalesce adjacent runs.
One used page must not force a whole 1-MiB transfer chunk onto disk. Keep
the format unchanged and bound every vhost ring and index cell to its
complete guest RAM span before passing addresses to the kernel.

Measure two unchanged samples first. Separate kernel startup, host asset and
OCI verification, guest read wait, hashing, mount, runtime configuration and
first completed exec. A stage called verification includes file-read time;
changing the hash cannot fix expensive memory backing or file transport.

Linux 6.18.44 also needed the upstream FUSE `bdi->io_pages = fc->max_pages`
backport to honor large explicit reads. A one-MiB transfer needs 256 data
pages plus four FUSE header descriptors; a 256-entry queue truncates the
negotiated limit. The host uses 512 entries, event-index notifications on
fresh and restored queues, and bounded worker-owned I/O buffers. Keep idle
wakeup, short/EOF/error payload and read-only regressions intact. Preserve
small-read speculation limits and workspace freshness.

Use the existing `capsem-bench` lifecycle, storage and crypto dimensions for
raw paired evidence. Do not add a benchmark crate or relax integrity,
resource defaults, or deadlines to make a result pass.
