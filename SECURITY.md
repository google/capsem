# Security Policy

## Supported versions

Security fixes land on the latest release on `main`. Older versions are not supported. Upgrade to the latest release to receive fixes.

## Reporting a vulnerability

Please report security issues **privately** via GitHub Security Advisories:

<https://github.com/google/capsem/security/advisories/new>

You can expect:

- An acknowledgement within 7 days.
- An initial assessment and severity classification within 14 days.
- A fix or mitigation plan within 90 days for high-severity issues.
- Public disclosure coordinated with the reporter after a fix is available.

Do **not** file public GitHub issues for security reports.

## Scope

Capsem is a security tool whose job is to isolate AI coding agents inside hardware-backed Linux VMs. The following are in scope:

- **Sandbox escape** -- any path for guest code to reach the host beyond the documented interfaces (vsock ports 5000--5005, VirtioFS mount, MITM proxy egress).
- **MITM proxy bypass** -- guest traffic that reaches the internet without passing through the policy engine.
- **Host-side privilege escalation** -- any way for a guest or local unprivileged user to elevate privileges on the host via the `capsem` service daemon, gateway, or per-VM process.
- **Credential / key exposure** -- disclosure of the MITM CA private key, Tauri updater signing key, or user session credentials.
- **Supply-chain integrity** -- tampering with downloaded VM assets (kernel, rootfs, initrd) undetected by the manifest hash check.
- **Gateway authentication bypass** -- unauthenticated calls that reach the service daemon via `capsem-gateway`.

## Out of scope

- Anything running inside the guest VM **by design**. The guest is an attacker-controlled environment; arbitrary code execution inside the guest is the whole point of the product.
- Vulnerabilities in third-party MCP servers configured by the user.
- Denial-of-service against a single VM (guests are cheap to re-create).
- Bugs in unreleased code on development branches.

## Disclosure

Once a fix is released, we will publish an advisory describing the issue, affected versions, and mitigation. Credit is given to reporters unless they request anonymity.
