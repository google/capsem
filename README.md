<p align="center">
  <img src="crates/capsem-app/icons/icon.svg" alt="Capsem" width="120" />
</p>

<h1 align="center">Capsem</h1>

<p align="center">
  Sandbox AI coding agents in hardware-isolated Linux VMs on macOS and Linux.<br/>
  Full network control, HTTPS inspection, MCP tool routing, headless browser automation, and per-session telemetry.
</p>

<p align="center">
  <a href="https://github.com/google/capsem/releases/latest"><img src="https://img.shields.io/github/v/release/google/capsem?label=download&color=blue" alt="Latest Release" /></a>
  <a href="https://codecov.io/gh/google/capsem"><img src="https://codecov.io/gh/google/capsem/graph/badge.svg" alt="Coverage" /></a>
  <a href="https://github.com/google/capsem/actions/workflows/ci.yaml"><img src="https://github.com/google/capsem/actions/workflows/ci.yaml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/google/capsem/blob/main/LICENSE"><img src="https://img.shields.io/github/license/google/capsem" alt="License" /></a>
</p>

## Install

```sh
curl -fsSL https://capsem.org/install.sh | sh
```

Pre-built binaries (DMG, .deb, .AppImage) are also available from the [latest release](https://github.com/google/capsem/releases/latest). See the [Getting Started](https://capsem.org/getting-started/) guide for details.

## Quick start

```sh
capsem uname -a
capsem echo hello
capsem 'ls -la /proc/cpuinfo'
```

## Documentation

Full documentation at **[capsem.org](https://capsem.org)**.

| Topic | Link |
|-------|------|
| Getting Started | [capsem.org/getting-started](https://capsem.org/getting-started/) |
| Architecture | [capsem.org/architecture/hypervisor](https://capsem.org/architecture/hypervisor/) |
| Security | [capsem.org/security/overview](https://capsem.org/security/overview/) |
| Custom Images | [capsem.org/architecture/custom-images](https://capsem.org/architecture/custom-images/) |
| Snapshots | [capsem.org/usage/snapshots](https://capsem.org/usage/snapshots/) |
| Benchmarks | [capsem.org/benchmarks/results](https://capsem.org/benchmarks/results/) |
| Troubleshooting | [capsem.org/debugging/troubleshooting](https://capsem.org/debugging/troubleshooting/) |
| Development | [capsem.org/development/getting-started](https://capsem.org/development/getting-started/) |
| Just Recipes | [capsem.org/development/just-recipes](https://capsem.org/development/just-recipes/) |
| Release Notes | [capsem.org/releases](https://capsem.org/releases/0-15/) |

## Disclaimer

This project is not an official Google project. It is not supported by Google and Google specifically disclaims all warranties as to its quality, merchantability, or fitness for a particular purpose.

## License

See [LICENSE](LICENSE).
