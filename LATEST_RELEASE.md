version: 1.5.1783866625
---
### Fixed
- Restored the complete canonical `just test` release gate on macOS and Linux
  in parallel for every stable and nightly tag; package builds and publication
  now depend on both current-tag gates, with no local, prior-run, or selected
  test subset accepted as release evidence.
- Installed the exact notarized macOS package and exact Linux package before
  making artifacts publishable, exercising the real native installers and
  post-install scripts. The public install, channel-switch, and upgrade glow-up
  remains the mandatory end-to-end post-deployment gate; the full `just test`
  gate runs once per operating system rather than being redundantly repeated.
- Documented the full two-OS test, exact-install, and public-glow-up invariant in
  the repository agent instructions and release/testing skills.
