UI decision: UI_NOT_PROVEN
User responsiveness verdict: not_tested
Authentication decision: AUTH_NOT_RUN
Host/guest: macOS arm64 / Apple VZ / arm64 Debian
Web end-to-end path proven: no
Failed UI gates: none; UI-0 through UI-12 remain open
Failed auth gates: not_run
Tranche 3 transport/UI legs feasible after Tranches 1 and 2: no
Tranche 3 OAuth/agent leg feasible: not_proven
Sanitized spike patch retained for later review: yes
Direct promotion without reimplementation/review/tests allowed: no

# Spike 0 result and evidence ledger

This document is the sanitized durable record for the Claude Web GUI
feasibility run on `v1.6`. It is intentionally updated while the spike runs.
Pending fields are not evidence and cannot be treated as passing gates.

## Candidate and host baseline

| Fact | Recorded value |
| --- | --- |
| Candidate commit at baseline | `572537db9a53f10c47cde0524c148db3266bdb45` |
| Branch | `v1.6` |
| Worktree at baseline | clean |
| Host | macOS 26.5.1, build 25F80, arm64 |
| Hardware | Mac17,6, Apple M5 Max, 18 CPU cores, 128 GiB RAM |
| GPU | Apple M5 Max, 40 cores, Metal 4 |
| Main display | 3840x2160, looks like 1920x1080 at 240 Hz |
| Secondary display | 2160x3840, looks like 1080x1920 at 240 Hz |
| Chrome | 150.0.7871.127 |
| Safari | 26.5 |
| Docker host | Colima 0.10.1, aarch64, 8 CPUs, 16 GiB RAM |
| Docker | client 29.4.2, server 29.2.1, Ubuntu 24.04.4 VM |
| Acceptance browser and viewport | pending first Web run |
| Local network row | loopback browser-to-gateway and Apple VZ host/guest; no shaping |

The branch is the user's explicit override. No detached worktree is used and
`main` is not touched. Secrets, vendor packages, generated VM images, raw
frames, raw logs, and reusable URLs or session material are excluded from git.

## Predeclared guest and run envelope

| Fact | Predeclared value |
| --- | --- |
| Profile source | `config/profiles/gui/profile.toml` |
| Author/build authority | `capsem-admin` only |
| Guest distribution | Debian 12 Bookworm, arm64 |
| Base image candidate | `debian:bookworm-slim` index `sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818` |
| Base arm64 manifest | `sha256:9b67294679b30e5d6ab257b40594feeb4a4b81f7fcf4131f4decf0d6a212a9b0` |
| Base image creation | 2026-07-13T00:00:00Z |
| Guest kernel build input | Capsem kernel 7.0.11, resolved by the current branch fallback |
| Kernel source SHA-256 | `e56c8356dda01136a6041c6ef832bd0ec99bd2d35dff97832aa5ec10ed014304` |
| VM resources | 4 vCPU, 12 GiB RAM, 64 GiB scratch unless measurement forces a recorded change |
| X display | Xdummy single-application display; exact pixel size pending first launch |
| Spike run/VM/app/trace ids | pending launch; opaque ids only |
| Idle deadline | 30 minutes without explicit user activity; no teardown before verdict while active |

The current global build config names a floating `debian:bookworm-slim` tag.
The digest above captures the baseline candidate, but the admin image plan must
either pin or explicitly ledger the resolved arm64 digest before UI-1 can pass.
It also names kernel branch `7.0`. The resolver found no non-EOL 7.0 release
and fell back to hardcoded 7.0.11. Kernel.org's signed checksum ledger provides
the SHA-256 above, but the current generated kernel Dockerfile downloads and
extracts the tarball without checking it. The spike must ledger the exact input;
the owning build rail needs checksum enforcement before this is release-grade.

## Pinned application inputs

### Claude Desktop

| Fact | Value |
| --- | --- |
| Origin | Anthropic signed stable Debian repository |
| Package | `claude-desktop` |
| Version | `1.22209.0` |
| Architecture | arm64 |
| Pool path | `pool/main/c/claude-desktop/claude-desktop_1.22209.0_arm64.deb` |
| Size | 158,507,044 bytes |
| SHA-256 | `7323fe6c3ab6b7078e81a9bf0200806e3486e73bc5873420ee9d26f10b66e1e9` |
| Signing fingerprint | `31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE` |
| Signature proof | GOOD in a clean Debian arm64 container |
| Signed metadata date | 2026-07-16 22:07:30 UTC |

The signer identity was `Anthropic Claude Code Release Signing
<security@anthropic.com>`. The hard dependencies include GTK3, NSS, DRM, GBM,
XCB DRI3, libsecret, and XDG desktop portal integration. QEMU and virtiofsd are
recommendations, not hard dependencies, and are excluded unless a measured
failure proves they are required.

### Xpra and HTML5 client

| Fact | Value |
| --- | --- |
| Origin | Xpra upstream Bookworm repository |
| Server/common/X11/codecs candidate | `6.5.1-r0-1` |
| HTML5 candidate | `21-r1-1` |
| HTML5 SHA-256 | `232d498314d302983522fbb8c9b6f91bd4ce12a9de2f37970ae3a7f5ff0ce466` |
| Signing fingerprint | `B4993B57323148E37977E5D873254CAD17978FAF` |
| Signature proof | GOOD in a clean Debian arm64 container |
| Signed metadata date | 2026-07-08 18:51:39 UTC |

The official deb822 source is Bookworm `main` at `https://xpra.org`, signed by
`/usr/share/keyrings/xpra.asc`, for `amd64 arm64`. Debian Bookworm's Xpra 3.x is
not accepted because upstream identifies it as unsupported and missing current
HTML5 and security fixes.

The guest pins the server-only subset below instead of installing the `xpra`
metapackage, which hard-depends on the unused GTK client. All rows come from
the signature-verified arm64 Bookworm package index.

| Guest package | Version | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `xpra-common` | `6.5.1-r0-1` | 7,734,396 | `ce68e85a976df7e2e34c5fe19be2a071c9595b554e22011bda33cf0f502ed58b` |
| `xpra-server` | `6.5.1-r0-1` | 375,700 | `28540b648a31ef8d469249ec859598e8bd0368ad5a4046e701c47ca51a8235d0` |
| `xpra-x11` | `6.5.1-r0-1` | 740,604 | `41c8ed007f6dafb32e3128dd3703afec9c52d76bea8b3a20af766a3199e762b6` |
| `xpra-codecs` | `6.5.1-r0-1` | 509,944 | `8669a2edcfc37854ba4cc4e8858215024399121ba23bae19a4a5115381d5f1d7` |

`xpra-html5` is a version-matched host/UI input, not a required guest runtime
package. Its signed `.deb` supplies the ignored spike client workspace. The
final image closure is still determined by the real admin build and recorded
from its OBOM, not inferred from these direct package rows.

## Dependency-closure experiment

A clean arm64 Bookworm apt simulation with `--no-install-recommends` selected
344 packages. The cause was not Xpra: Claude declares a dependency alternative
whose first satisfier is `kde-cli-tools`, so apt selected the KDE closure.

Adding `trash-cli` explicitly in the same transaction satisfies that
alternative without KDE. Adding the required `xserver-xorg-video-dummy` and
`dbus-x11` at the same time produces the current 200-package closure candidate.
That simulation used the broad Xpra metapackage. The selected server-only set
above should reduce the closure further. The actual package/version OBOM,
installed bytes, rootfs bytes, and delta from the admin-built baseline remain
pending S01-003. Simulation is not build proof.

No desktop environment, window manager, guest browser, VNC/noVNC, Spice,
socat, websockify, QEMU, or virtiofsd is admitted by this decision.

## Xpra/vsock capability findings

A disposable arm64 Debian diagnostic installed Xpra `v6.5.1-r0` and established
that:

- `xpra showconfig` exposes `bind-vsock` and `vsock-auth`;
- Python exposes `socket.AF_VSOCK = 40`;
- the Xpra HTML5 `index.html`, `Client.js`, and `Protocol.js` are installed;
- the `xpra.net.vsock` implementation is present;
- upstream defines `--bind-vsock` as `[CID]:[PORT]`, maps `auto` and `any` to
  `CID_ANY`, and opens an `AF_VSOCK` `SOCK_STREAM` listener.

The candidate guest endpoint is therefore `any:<fixed-product-port>`. This is
capability evidence only. It must be proven inside the admin-built GUI VM with
authentication and without a guest TCP listener.

## Apple VZ implementation seam

The current Capsem Apple VZ backend registers host listeners and accepts
guest-initiated connections. It does not yet expose host-initiated guest
connections. The already-linked `objc2-virtualization` 0.3.2 binding exposes
`VZVirtioSocketDevice::connectToPort_completionHandler`, returning a
`VZVirtioSocketConnection` with a file descriptor. The smallest expected
Tranche 3 seam is a typed, main-thread-safe connection method owned by
`capsem-process`, retaining the VZ connection object for fd lifetime and
addressing only the product-selected GUI port. A generic caller-selected proxy
is forbidden.

This means direct Apple-VZ host-to-guest transport appears implementable
without TCP, but it remains unproven until a focused test connects to Xpra in
the actual GUI VM.

## Predeclared Stage A budgets

| Measurement | Acceptance threshold |
| --- | --- |
| Process start to first decoded Web frame | 15 seconds maximum |
| Established input to visible acknowledgement | 150 ms p95 local |
| Resize request to stable frame | 500 ms p95 local |
| Reconnect request to resumed frame | 3 seconds maximum |
| Sustained-scroll presentation | at least 24 FPS median; under 5% intervals over 100 ms |
| Sustained resize | at least 20 presented FPS median |
| Input correctness | zero dropped or stuck inputs in the manual checklist |
| Relay queues | bounded, with cap occupancy/time reported |
| Resource stability | no OOM, swap storm, unbounded growth, or continuous idle CPU saturation |
| CPU/memory evidence | mandatory; missing evidence yields `UI_NOT_PROVEN` |

These are feasibility thresholds, not product SLOs.

## Gate ledger

| Gate | State | Evidence or next proof |
| --- | --- | --- |
| UI-0 Provenance | pending | signed metadata verified; admin build must verify the exact downloaded package |
| UI-1 Admin-authored profile | pass | Admin create, validate, check, build, materialize, and Apple VZ boot of `gui` |
| UI-2 Single application | pending | Claude window under Xpra/Xdummy, no WM |
| UI-3 Direct vsock | pending | Xpra AF_VSOCK to typed Apple VZ relay, no TCP |
| UI-4 Gateway path | pending | authenticated gateway/service/process ownership chain |
| UI-5 Capsem Web surface | pending | normal Capsem UI sandboxed iframe |
| UI-6 Interaction | pending | complete manual interaction checklist |
| UI-7 Responsiveness | pending | measurements plus explicit user verdict |
| UI-8 CPU/memory | pending | per-component launch/active/idle/reconnect/stop samples |
| UI-9 FPS/bandwidth | pending | live presented FPS, Xpra updates, jank, codec, queues, bytes |
| UI-10 Stability | pending | bounded run including idle and reconnect |
| UI-11 Observability | pending | one opaque correlated run identity |
| UI-12 Cleanup | pending | post-verdict removal and prohibited-data audit |

## Build attempts

### Attempt 1 — failed after rootfs construction

`just build-assets gui arm64 tmp/gui/spike-0/assets` reached the normal
`capsem-admin image build` backend. It built the 7.0.11 kernel/initrd, rendered
the GUI profile without language-runtime installer layers, verified the vendor
keys and direct package SHA-256 values, installed the exact Claude/Xpra set,
and completed the rootfs container image. Evidence extraction then failed
before rootfs export because `extract_software_inventory` unconditionally ran
`python3 -m pip list` even though the profile declared no Python package set.

This is an Admin/build evidence-boundary defect, not a GUI-package failure.
The forward fix makes inventory query only the package managers declared by
the active profile and adds a regression test. Attempt 1 is not build or boot
proof; its partial output is replaced by the next clean Admin run.

### Attempt 2 — image succeeded; OBOM rejected as host-contaminated

The clean Admin rerun completed at candidate `11327e6d`, generated manifest
release `2026.0717.1`, and produced these arm64 outputs:

| Output | Bytes | BLAKE3 |
| --- | ---: | --- |
| `vmlinuz` | 8,786,432 | `af8e3b893ae19b2776fe5a2f6c5a25a2e49086de343b1062f5e9de168da96363` |
| `initrd.img` | 996,564 | `b8776974ed4b97044a3356275c3f30d2c47c3f5e967eb9018a613ae1f9ab86ac` |
| `rootfs.erofs` | 612,360,192 | `c8cc0bbdb81cf923be4e3793630b9915b05ce8cb408203f6a775d73a6893a158` |
| `software-inventory.json` | 45,715 | `a61ad9f33f8a8dcd9865bfea10711ea770342f5809c41150456e6bff129cc107` |

The Admin inventory contains 356 dpkg packages. It records
`claude-desktop=1.22209.0` and the four direct Xpra server packages at
`6.5.1-r0-1`; the profile's forbidden browser, desktop, window-manager,
VNC, websockify, socat, and SSH-server packages are absent.

The generated `obom.cdx.json` cannot be accepted. Although the command was
given the extracted guest rootfs path, `cdxgen -t os` means live-host
inventory: it emitted the macOS 26.5.1 host, 2,794 host components, local
Claude.app rows, and zero `pkg:deb/` components. The manifest therefore pins
a structurally valid but semantically false VM OBOM. The forward fix uses
cdxgen's offline-rootfs mode (`-t rootfs`) and makes Admin reject an OBOM with
no Debian package components. Attempt 2 proves Admin image construction and
the independent dpkg inventory, but not valid OBOM production or VM boot.

### Attempt 3 — valid Admin evidence and Apple VZ boot

Candidate `19e35b85` completed the clean Admin build with asset release
`2026.0717.2`. Validation compiled 10 profile rules; source check found all six
profile payloads; materialization pinned the Admin outputs below:

| Output | Bytes | BLAKE3 |
| --- | ---: | --- |
| `vmlinuz` | 8,786,432 | `af8e3b893ae19b2776fe5a2f6c5a25a2e49086de343b1062f5e9de168da96363` |
| `initrd.img` | 996,564 | `b8776974ed4b97044a3356275c3f30d2c47c3f5e967eb9018a613ae1f9ab86ac` |
| `rootfs.erofs` | 612,360,192 | `a253dece33a47e81053785dab5d72bc2159a2e86bef7d19150a5919388eab203` |
| `obom.cdx.json` | 17,912,554 | `adf98604bbedf6b546c335f78bb9f32063ef77be4e8bc6a3b411441192fb4e2d` |
| `software-inventory.json` | 45,715 | `a61ad9f33f8a8dcd9865bfea10711ea770342f5809c41150456e6bff129cc107` |

The cdxgen 12.7.1 OBOM identifies `rootfs` as its container component and
contains 16,090 components, including 548 `pkg:deb/` binary/source records.
The independent installed-package inventory remains 356 dpkg rows. The build
ledger records a 1,159,753,216-byte exported rootfs tar; the 612,360,192-byte
EROFS is 547,393,024 bytes smaller (47.2% reduction, 52.8% of the tar size).
No code-profile image was available locally for a cross-profile size delta,
so none is invented.

An isolated signed service loaded the Admin-materialized profiles directory
and exact Admin asset directory. `POST /vms/create` with `profile_id=gui`
returned Running VM `e452d927-4b1a-4290-8787-6a72203330ba`. In-guest proof:

- Debian GNU/Linux 12 Bookworm, arm64, kernel 7.0.11;
- `claude-desktop=1.22209.0` and `xpra-{common,server,x11,codecs}=6.5.1-r0-1`;
- `/usr/bin/claude-desktop` and `/usr/bin/xpra` present;
- Xpra exposes `bind-vsock` and `vsock-auth` configuration keys;
- every forbidden browser, desktop, window-manager, VNC, websockify, socat,
  and SSH-server package is absent or has dpkg status `not-installed`;
- listeners are restricted to Capsem's loopback DNS proxy on 1053 and network
  proxy on 10080/10443 before the application/Xpra launch tranche.

UI-1 passes. This does not prove a Claude window, relay, browser surface,
interaction, performance, stability, or authentication; UI-2 through UI-12
remain open.

## Measurement tables

No runtime samples exist yet. Rows are added only from the complete Capsem Web
path; disposable-container diagnostics do not populate acceptance tables.

| Workload | Input-to-update p50/p95/p99 | Presented FPS median | >100 ms intervals | WS RTT | Notes |
| --- | --- | --- | --- | --- | --- |
| idle | pending | pending | pending | pending | |
| typing | pending | pending | pending | pending | |
| scrolling | pending | pending | pending | pending | |
| dialog | pending | pending | pending | pending | |
| resize | pending | pending | pending | pending | |
| reconnect | pending | pending | pending | pending | |

| Component | Baseline | First frame | Peak interaction | Five-minute idle | Reconnect | Post-stop |
| --- | --- | --- | --- | --- | --- | --- |
| Claude CPU/RSS/PSS | pending | pending | pending | pending | pending | pending |
| Xpra/Xorg CPU/RSS/PSS | pending | pending | pending | pending | pending | pending |
| VM/helper RSS | pending | pending | pending | pending | pending | pending |
| capsem-process CPU/RSS | pending | pending | pending | pending | pending | pending |
| capsem-gateway CPU/RSS | pending | pending | pending | pending | pending | pending |
| browser renderer RSS | pending | pending | pending | pending | pending | pending |
| relay queue bytes | pending | pending | pending | pending | pending | pending |

## Failed approaches and lessons

- A zsh diagnostic loop used `path` as its variable name. In zsh, `$path` is
  the special command-search array, so this made `curl` and `rg` appear absent.
  Renaming the variable to `source_file` fixed the run. This was shell-state
  corruption, not a missing dependency.
- A naive `--no-install-recommends` assumption did not produce a minimal
  closure because apt dependency-alternative ordering selected KDE. The
  explicit `trash-cli` satisfier is part of the profile design.
- The configured kernel branch is EOL and resolves through a hardcoded fallback,
  while the build Dockerfile does not verify kernel.org's published checksum.
  The exact 7.0.11 digest is recorded, but this is a build-rail weakness rather
  than a property the GUI profile can conceal.
- The first Admin build exposed an unconditional pip-inventory query after the
  GUI rootfs completed. A profile with no Python package set must produce an
  apt-only inventory; the failed attempt remains recorded above.
- The second Admin build exposed a semantic OBOM bug: `cdxgen -t os` ignores
  an extracted-rootfs operand for inventory ownership and scans the live host.
  Offline guest roots require `-t rootfs`; shape-only validation was too weak
  to catch the macOS document and now requires Debian package components.
- The v1.6 Admin CLI accepts the profile path positionally for `validate` and
  `check`, but `materialize` uses `--profile`. The first combined invocation
  used the materialize spelling for validate and was rejected before mutation.
- A guest forbidden-package probe initially treated any `dpkg-query -W`
  record as installed; dpkg retains `unknown ok not-installed` rows. The
  acceptance predicate must require the exact status `install ok installed`.
  The first corrected command also needed `\${Status}` so the guest shell did
  not expand dpkg's format token under `set -u`.
- Attempts 2 and 3 produced identical-size 612,360,192-byte EROFS files but
  different hashes. The profile inputs and installed inventory were stable,
  but byte-for-byte rootfs reproducibility is not yet established.
- Reusing the `code` profile, generating a backend outside Capsem Admin, or
  creating a one-off GUI Dockerfile would not prove the product architecture
  and is prohibited by the spike contract.

## Retained work and tranche ownership

| Retained component | Owner | Required before product use |
| --- | --- | --- |
| `gui` source profile and input pins | Tranche 1/3 | schema review, reproducibility, OBOM and release graph gates |
| typed app/Xpra lifecycle | Tranche 2/3 | lifecycle, cancellation, security, audit and failure tests |
| Apple VZ guest connection | Tranche 3 | backend tests and Linux/rust-vmm bake-off |
| process relay and gateway routes | Tranche 3 | auth/origin/ownership/backpressure/failure tests |
| Web GUI component and metrics | Tranche 3 | design-system, accessibility, frontend and browser verification |

## Smallest next experiment

Launch Xdummy, Xpra, and Claude Desktop in the proven GUI VM without a window
manager. Capture the exact lifecycle commands, first-window evidence, process
tree, sockets, and clean-stop result before implementing a relay or frontend.

## User verdict, authentication, and teardown

User responsiveness notes: pending complete Capsem Web run.

Authentication remains deliberately `AUTH_NOT_RUN` until Stage A produces a
browser-visible Claude UI and the user elects to continue.

Teardown proof: pending. The isolated UI-1 image-proof VM is running; no GUI
application process or authentication run has started yet.
