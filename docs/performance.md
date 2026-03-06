# Capsem Performance

Benchmarks for the Capsem VM sandbox: disk I/O, rootfs reads, CLI startup, HTTP latency, and MITM proxy throughput.

## Running benchmarks

```bash
just bench                        # all benchmarks (boots VM once)
just run "capsem-bench throughput" # proxy throughput only
just run "capsem-bench disk"       # scratch disk I/O only
just run "capsem-bench http"       # HTTP latency only
just run "capsem-bench startup"    # CLI cold-start only
```

The `just bench` recipe is part of `just full-test`.

## Benchmark suite (`capsem-bench`)

| Mode | What it measures |
|------|-----------------|
| `disk` | Scratch disk seq/rand read+write (256 MB default, ext4 on virtio-blk) |
| `rootfs` | Rootfs seq/rand read (squashfs via virtio-blk, read-only) |
| `startup` | Cold-start latency for python3, node, claude, gemini, codex |
| `http` | HTTP request latency + throughput (ab-style, concurrent GETs through proxy) |
| `throughput` | 100 MB download through the full MITM proxy pipeline |

Output: rich table to stderr (human), JSON to stdout (machine).

## MITM proxy throughput

Tests the complete data path:

```
guest curl -> iptables REDIRECT -> capsem-net-proxy (TCP 10443)
  -> vsock (port 5002) -> host MITM proxy
  -> TLS termination + policy check + upstream TLS
  -> ash-speed.hetzner.com -> back
```

### Baseline (M-series Mac, 2026-03-06)

| Metric | Value |
|--------|-------|
| File | 100 MB (`ash-speed.hetzner.com/100MB.bin`) |
| Duration | 2.86s |
| Throughput | **34.9 MB/s** |

Host-side Rust test (`mitm_proxy_download_throughput`): **30.3 MB/s** — confirms the vsock + guest-side relay overhead is minimal (~15%).

### Running the proxy throughput test

In-VM (capsem-bench):
```bash
just run "capsem-bench throughput"
```

Host-side Rust (skipped by default, requires internet):
```bash
cargo test -p capsem-core --test mitm_integration -- --ignored mitm_proxy_download_throughput --nocapture
```

In-VM capsem-doctor (skips if domain not in allow list):
```bash
just run "capsem-doctor -k throughput"
```

### Domain allow list

`ash-speed.hetzner.com` must be in the allow list. It is included by default in:
- `config/defaults.toml` (`network.custom_allow`)
- `config/integration-test-user.toml`

For personal use, verify `~/.capsem/user.toml` includes it:
```toml
[settings."network.custom_allow"]
value = "elie.net, *.elie.net, ash-speed.hetzner.com"
```

## Disk I/O

Scratch disk is a fresh ext4 volume on virtio-blk, formatted at every boot. Upper tmpfs overlay writes go here.

Expected ranges on Apple Silicon (M-series):

| Test | Typical |
|------|---------|
| Seq write 1MB | 400–600 MB/s |
| Seq read 1MB | 500–800 MB/s |
| Rand write 4K | 1–5 MB/s, 300–1000 IOPS |
| Rand read 4K | 5–20 MB/s, 1000–5000 IOPS |

## CLI startup latency

Measures cold-start (drop_caches before each run, 3 runs each).

Expected ranges:

| CLI | Typical |
|-----|---------|
| python3 | 20–60 ms |
| node | 30–80 ms |
| claude | 200–600 ms |
| gemini | 150–500 ms |
