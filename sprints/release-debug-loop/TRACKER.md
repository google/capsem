# Release Debug Loop Tracker

## Queue

- [ ] B1 - VM assets missing / install setup chain broken (fix implemented; live install gate pending sudo-capable terminal)
- [ ] B2 - AI provider onboarding/settings parsing broken
- [ ] B3 - VM list/session UI broken after launch

## Current Focus

B1 - Automated proof and fix landed for setup completing after failed asset download. Remaining B1 gate is a live `just install` run from a terminal where the macOS sudo password can be entered.

## Loop Template

For each bug:

- [ ] Confirm reproduction
- [ ] Trace chain
- [ ] Root cause written
- [ ] Failing proof added
- [ ] Fix implemented
- [ ] Verification run
- [ ] Changelog updated
- [ ] Commit or explicit no-commit note

## Evidence Log

- 2026-05-13: Screenshots show onboarding assets missing and Providers step empty.
- 2026-05-13: User reports same class of asset/network failure on another Mac after release install.
- 2026-05-13: Local `just install` hard-clean live run stopped at sudo password; `~/.capsem` absent afterwards.
- 2026-05-13: Current local state later had `~/.capsem/assets/arm64/{vmlinuz,initrd.img,rootfs.squashfs}` present and `capsem status` reported assets OK, but service/gateway runtime files were stale (`capsem status` said Running=false; gateway port 19222 refused connections).
- 2026-05-13: B1 trace found package payload seeds only signed manifests, postinstall calls `capsem setup --non-interactive --accept-detected`, and setup marked `welcome` complete before the asset download succeeded.
- 2026-05-13: Red proof added: `tests/capsem-install/test_setup_wizard.py::test_setup_fails_when_required_assets_cannot_download` simulated a manifest-only install with `CAPSEM_RELEASE_URL=http://127.0.0.1:9`; before the fix setup exited 0, printed `Setup complete`, and would persist completed install state despite missing VM assets.
- 2026-05-13: Fix implemented in `crates/capsem/src/setup.rs`: asset download failure now aborts setup, and `welcome` is persisted only after the download task succeeds.
- 2026-05-13: Verification passed: `uv run pytest tests/capsem-install/test_setup_wizard.py::test_setup_fails_when_required_assets_cannot_download -q`; `cargo test -p capsem setup -- --nocapture`.

## B1 Loop State

- [x] Confirm reproduction -- red packaging test proved setup completed after failed VM asset download.
- [x] Trace chain -- package manifest-only payload -> postinstall setup -> `step_welcome` background download -> swallowed error -> `install_completed=true`.
- [x] Root cause written -- setup marked `welcome` done before asset readiness and converted download failure into a warning.
- [x] Failing proof added -- `test_setup_fails_when_required_assets_cannot_download`.
- [x] Fix implemented -- setup now fails loudly and only marks `welcome` after successful asset download.
- [x] Verification run -- targeted Python proof and Rust setup tests passed.
- [x] Changelog updated -- `CHANGELOG.md` Unreleased Fixed.
- [x] Commit or explicit no-commit note -- no commit yet because the worktree already contains unrelated uncommitted changes from prior install/debug-report work.

## Commands To Start B1

```bash
capsem version
capsem status
ls -la ~/.capsem ~/.capsem/assets ~/.capsem/assets/$(uname -m)
curl -fsS http://127.0.0.1:19222/status
capsem run 'set -eux; cat /etc/resolv.conf; getent hosts elie.net; curl -fsS --connect-timeout 10 https://elie.net >/dev/null'
```

If local install is absent, first rerun:

```bash
just install
```

Run it in a terminal where the macOS sudo password can be entered.
