<p align="center">
  <img src="crates/capsem-app/icons/icon.svg" alt="Capsem" width="120" />
</p>

<h1 align="center">Capsem</h1>

<p align="center">
  <strong>The fastest way to ship with AI securely.</strong><br/>
  Sandbox AI coding agents in hardware-isolated Linux VMs on macOS and Linux.<br/>
  Full network control, HTTPS inspection, MCP tool routing, and per-session telemetry.
</p>

<p align="center">
  <a href="https://codecov.io/gh/google/capsem"><img src="https://codecov.io/gh/google/capsem/graph/badge.svg" alt="Coverage" /></a>
  <a href="https://github.com/google/capsem/actions/workflows/ci.yaml"><img src="https://github.com/google/capsem/actions/workflows/ci.yaml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/google/capsem/blob/main/LICENSE"><img src="https://img.shields.io/github/license/google/capsem" alt="License" /></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-14%2B-000000?logo=apple&logoColor=white" alt="macOS 14 or later" />
  <img src="https://img.shields.io/badge/Ubuntu-24.04%2B-E95420?logo=ubuntu&logoColor=white" alt="Ubuntu 24.04 or later" />
  <img src="https://img.shields.io/badge/Debian-13%2B-A81D33?logo=debian&logoColor=white" alt="Debian 13 or later" />
</p>

## Capsem 0.6 pre-release

Capsem 0.6 is undergoing release qualification. Installation instructions,
downloadable packages, and detailed documentation are intentionally unavailable
until the public release, planned for Summer 2026.

The current documentation status is at **[docs.capsem.org](https://docs.capsem.org/)**.

## Supported platforms

| System | Versions supported | Hardware |
|---|---|---|
| macOS | 14 (Sonoma) or later | Apple Silicon (M1 or newer) |
| Ubuntu | 24.04 or later | x86_64 or arm64, KVM capable |
| Debian | 13 or later | x86_64 or arm64, KVM capable |

Capsem aims to run on every currently supported version of these operating
systems. Not covered yet: Ubuntu 22.04 and Debian 12
([#181](https://github.com/google/capsem/issues/181)), Alpine
([#182](https://github.com/google/capsem/issues/182)).

Every release above is proved each gate run — the package is unpacked and run
on each one, and must be refused on the rest.

## Disclaimer

This project is not an official Google project. It is not supported by Google and Google specifically disclaims all warranties as to its quality, merchantability, or fitness for a particular purpose.

## License

See [LICENSE](LICENSE).
