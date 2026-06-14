# Benchmark Ledger: Kernel 7.0 + EROFS zstd

Date: 2026-06-04

## Kernel Build Proof

Command:

```bash
uv run capsem-builder build guest/ --arch arm64 --template kernel \
  --output target/kernel7-smoke --kernel-version 7.0.11
```

Result:

| Artifact | Size |
| --- | ---: |
| `target/kernel7-smoke/arm64/vmlinuz` | 8,585,728 bytes |
| `target/kernel7-smoke/arm64/initrd.img` | 995,125 bytes |

Notes:

- `make olddefconfig` accepted the arm64 defconfig against Linux `7.0.11`.
- `make Image` completed and produced `arch/arm64/boot/Image`.
- Build log compiled the zstd decompressor path.

## Rootfs Build Proof

Command:

```bash
CAPSEM_BUILD_EXPERIMENTAL_EROFS=1 \
CAPSEM_BUILD_EROFS_COMPRESSION=zstd \
CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL=15 \
CAPSEM_BUILD_EROFS_CLUSTER_SIZE=65536 \
uv run capsem-builder build guest/ --arch arm64 --template rootfs \
  --output target/rootfs-erofs-zstd
```

Result:

| Artifact | Size |
| --- | ---: |
| `target/rootfs-erofs-zstd/arm64/rootfs.squashfs` | 480,788,480 bytes |
| `target/rootfs-erofs-zstd/arm64/rootfs.erofs` | 774,451,200 bytes |

Interpretation:

- The recorded EROFS artifact came from the initial default-level zstd run.
  The builder now defaults EROFS zstd to level 15; rerun this row before
  comparing artifact size.
- EROFS zstd is larger than squashfs zstd on the current full rootfs:
  `+293,662,720 bytes` (`+61.08%`).
- Runtime speed remains the decision point: compare rootfs read and startup
  behavior inside the `7.0.11` guest.

## Mount Proof

Host/container mount smoke:

```bash
docker run --rm --privileged -v "$PWD/target/rootfs-erofs-zstd/arm64:/assets" \
  debian:trixie-slim bash -c '... mount -t erofs -o loop,ro /assets/rootfs.erofs /mnt/rootfs ...'
```

Result:

- Failed with `fsconfig() failed: Operation not supported`.
- This is a host-kernel limitation in the container environment, not an image
  creation failure. Full mount/runtime proof must run inside the Capsem
  `7.0.11` guest kernel or on Linux/KVM hardware with EROFS zstd enabled.

## Runtime Speed Numbers

All runtime measurements below booted the arm64 Capsem guest with kernel
`7.0.11` and ran the same command through `capsem run` against isolated
service homes under `target/speed-runs/`.

Lanes:

- `kernel7-squashfs`: `rootfs.squashfs`, zstd squashfs baseline.
- `kernel7-erofs-zstd`: `rootfs.erofs`, zstd level 15, 64 KiB cluster.
- `kernel7-erofs-lz4hc`: `rootfs.erofs`, lz4hc level 12, 64 KiB cluster.

Command shape:

```bash
capsem --uds-path "$sock" run --timeout 600 \
  'sh -lc '"'"'uname -r; capsem-bench rootfs; capsem-bench startup'"'"''
```

Result means are over three fresh VM runs per lane. Size is recorded only as
context; speed is the decision metric.

| Lane | Rootfs size | Fresh run mean | Fresh run min-max | Seq read mean | Rand read mean | Node startup mean | Codex startup mean |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| squashfs zstd | 458.5 MiB | 9.10 s | 8.79-9.73 s | 599.3 MB/s | 7,757 IOPS | 130.6 ms | 305.2 ms |
| EROFS zstd-15 | 562.7 MiB | 6.58 s | 6.45-6.77 s | 1,567.2 MB/s | 19,857 IOPS | 36.4 ms | 131.7 ms |
| EROFS lz4hc-12 | 720.5 MiB | 6.05 s | 5.92-6.29 s | 4,316.7 MB/s | 28,235 IOPS | 18.5 ms | 78.1 ms |

Interpretation:

- EROFS lz4hc-12 is the speed winner on this Mac/arm64 run.
- EROFS zstd-15 is still materially faster than squashfs, but slower than
  lz4hc for both sequential rootfs reads and cold CLI startup.
- The guest reports `/` as overlay, so the proof is lane selection plus
  successful `7.0.11` guest execution, not a direct `findmnt /` filesystem
  label.

## Remaining Validation

- Re-run the same lanes on Linux/KVM x86_64 hardware before accepting Linux
  production defaults.
- Promote the winning EROFS lane into the hash-addressed asset manifest path
  before using it outside benchmark/dev mode.

## Network Validation

Initial kernel7 network smoke on the EROFS lz4hc-12 lane failed before the
network could be trusted:

```text
iptables v1.8.9 (legacy): can't initialize iptables table `nat': Table does not exist
```

The temporary diagnosis was that Linux 7.0 split legacy iptables behind
`CONFIG_NETFILTER_XTABLES_LEGACY` and `CONFIG_IP_NF_IPTABLES_LEGACY`, but the
final decision is not to carry that legacy table path. The accepted path is
`iptables-nft -t nat -S`.

Final nft path:

- Kernel defconfigs enable nf_tables plus the compatibility objects required
  by the actual `iptables-nft` command syntax:
  `CONFIG_NF_TABLES`, `CONFIG_NF_TABLES_IPV4`, `CONFIG_NFT_NAT`,
  `CONFIG_NFT_REDIR`, `CONFIG_NETFILTER_XTABLES`, `CONFIG_NFT_COMPAT`, and
  `CONFIG_NETFILTER_XT_TARGET_REDIRECT`.
- Kernel defconfigs forbid the legacy table engine:
  `CONFIG_NETFILTER_XTABLES_LEGACY`, `CONFIG_IP_NF_IPTABLES_LEGACY`,
  `CONFIG_IP_NF_IPTABLES`, `CONFIG_IP_NF_NAT`, and
  `CONFIG_IP_NF_TARGET_REDIRECT`.
- `capsem-init` uses `IPTABLES=iptables-nft` only and exits fatal if any NAT
  rule append fails.
- The rootfs template removes Debian's accidental legacy frontend binaries:
  `/usr/sbin/iptables-legacy*` and `/usr/sbin/ip6tables-legacy*`.

The first nft rebuild still failed in the guest because the kernel lacked the
iptables-nft extension compatibility objects:

```text
Warning: Extension udp revision 0 not supported, missing kernel module?
Warning: Extension REDIRECT revision 0 not supported, missing kernel module?
iptables v1.8.9 (nf_tables): RULE_APPEND failed (No such file or directory): rule in chain OUTPUT
```

The fixed rebuild compiled the required nft/xt compatibility objects:
`nft_compat.o`, `x_tables.o`, `xt_tcpudp.o`, `xt_REDIRECT.o`, `nft_nat.o`,
`nft_redir.o`, and `nft_chain_nat.o`.

The final nft lane uses:

- `target/speed-assets/kernel7-erofs-lz4hc-nft/vmlinuz`
- `target/speed-assets/kernel7-erofs-lz4hc-nft/initrd.img`
- `target/speed-assets/kernel7-erofs-lz4hc-nft/rootfs.erofs`

Guest NAT proof:

```text
KERNEL=7.0.11
NAT_RULES_START
-P PREROUTING ACCEPT
-P INPUT ACCEPT
-P OUTPUT ACCEPT
-P POSTROUTING ACCEPT
-A OUTPUT -p udp -m udp --dport 53 -j REDIRECT --to-ports 1053
-A OUTPUT -p tcp -m tcp --dport 53 -j REDIRECT --to-ports 1053
-A OUTPUT -p tcp -m tcp --dport 443 -j REDIRECT --to-ports 10443
-A OUTPUT -p tcp -m tcp --dport 80 -j REDIRECT --to-ports 10080
-A OUTPUT -p tcp -m tcp --dport 11434 -j REDIRECT --to-ports 10080
NAT_RULES_END
legacy-absent
2001:4860:4826:7700:: www.google.com
2001:4860:482c:7700:: www.google.com
HTTP/1.1 200 OK
```

Quick historical network numbers below were gathered on the temporary legacy
netfix lane before the nft cutover. They are useful as rough ballpark only and
must be rerun on the final nft lane before using them as benchmark-grade
numbers.

| Benchmark | Result |
| --- | ---: |
| HTTP `https://www.google.com/`, 50 req, c=5 | 50/50 success, 58.4 rps |
| HTTP latency | p50 64.3 ms, p95 213.0 ms, p99 215.5 ms |
| Proxy throughput, 9.98 MB PDF | 20.61 MB/s, 0.462 s |

DNS load used `CAPSEM_BENCH_DNS_DURATION=5`:

| Concurrency | RPS | p50 | p95 | p99 | Errors |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 3,442.4 | 0.3 ms | 0.3 ms | 0.3 ms | 0 |
| 10 | 12,322.4 | 0.8 ms | 1.0 ms | 1.2 ms | 0 |
| 50 | 9,954.4 | 4.1 ms | 8.1 ms | 12.5 ms | 0 |
| 200 | 9,681.0 | 19.2 ms | 24.0 ms | 26.4 ms | 0 |

MCP load used `CAPSEM_BENCH_MCP_DURATION=5`:

| Concurrency | RPS | p50 | p95 | p99 | Errors |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1,882.2 | 0.5 ms | 0.7 ms | 0.9 ms | 0 |
| 10 | 7,569.2 | 1.3 ms | 1.8 ms | 2.0 ms | 0 |
| 50 | 7,989.8 | 6.1 ms | 7.5 ms | 8.8 ms | 0 |
| 200 | 7,722.6 | 25.8 ms | 28.1 ms | 29.5 ms | 0 |

MITM load used `CAPSEM_BENCH_MITM_DURATION=5` against the standard synthetic
nonexistent target. The harness reports every request as an error for this
target; the latency/rps values are the useful failure-path proxy numbers.

| Concurrency | RPS | p50 | p95 | p99 |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1,764.2 | 0.5 ms | 0.7 ms | 0.9 ms |
| 10 | 3,484.6 | 2.7 ms | 4.9 ms | 6.3 ms |
| 50 | 3,484.2 | 13.0 ms | 27.4 ms | 35.7 ms |
| 200 | 2,992.6 | 44.3 ms | 120.9 ms | 169.2 ms |

## Mac/VZ DAX Probe

Linux's DAX lane on `origin/main` uses a separate virtio-pmem transport
(`/dev/pmem0`) and mounts EROFS with `-o dax`. Apple VZ does not expose a
virtio-pmem device in the local Virtualization.framework bindings, so the Mac
probe tested the closest possible path: same EROFS lz4hc-12 image, same VZ
rootfs block device, but init attempted `mount -t erofs -o ro,dax /dev/vda`.

Command shape:

```bash
CAPSEM_EXPERIMENTAL_EROFS=1 \
CAPSEM_EXPERIMENTAL_EROFS_DAX=1 \
capsem --uds-path "$sock" run --timeout 600 '...'
```

Result:

```text
[capsem-init] mounting /dev/vda (erofs-dax, opts=ro,dax)...
erofs: dax options not supported
mount: mounting /dev/vda on /mnt/a failed: Invalid argument
[capsem-init] FATAL: cannot mount /dev/vda as erofs-dax
```

Interpretation:

- Mac/VZ cannot use the Linux DAX win through the existing VZ virtio-blk
  rootfs transport.
- The updated initrd still boots the lz4hc EROFS lane when DAX is disabled:
  one sanity run completed in `6.00 s`, with sequential rootfs read
  `4,117.1 MB/s` and Codex startup mean `77.8 ms`.
- To get true DAX on Mac, we would need an Apple-exposed pmem/DAX-capable
  transport. The local bindings do not provide one.
