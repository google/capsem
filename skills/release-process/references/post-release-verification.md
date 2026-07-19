# Post-Release Verification

Reference for /release-process: public release verification, glow-up checks, binary transition proof, demo-facing installer proof.


```bash
gh release view vX.Y.Z
gh release download vX.Y.Z --pattern '*.pkg' -D /tmp/verify
pkgutil --check-signature /tmp/verify/Capsem-*.pkg
spctl -a -vv -t install /tmp/verify/Capsem-*.pkg      # Gatekeeper accepts notarized+stapled
xcrun stapler validate /tmp/verify/Capsem-*.pkg       # Staple ticket present
gh release download vX.Y.Z --pattern '*.deb' -D /tmp/verify
curl -fsSL https://release.capsem.org/channels.json -o /tmp/verify/channels.json
curl -fsSL https://release.capsem.org/assets/stable/manifest.json -o /tmp/verify/asset-manifest.json
uv run python3 scripts/check-public-binary-release.py \
  --channel stable \
  --manifest-url https://release.capsem.org/assets/stable/manifest.json \
  --install-script-url https://capsem.org/install.sh \
  --docker-linux-install \
  --docker-channel-switch \
  --docker-upgrade \
  --docker-transition-from-manifest /path/to/frozen-predeploy-manifest.json
```

Use `scripts/check-public-binary-release.py` for post-deploy glow-up instead of
ad hoc `tar`/`strings` checks. It validates public `install.sh`, package URLs,
package SHA-256, package-owned binary hashes, absence of packaged
`assets/manifest.json`, `manifest-metadata.json` source provenance, Docker
install, stable/nightly asset switching, and the binary updater path. Package
scripts must not normalize or convert manifest JSON; the selected channel
manifest is the only runtime manifest format.

Binary transition proof must use two manifests that reference two genuinely
compiled package cohorts with different versions. Rewriting only Debian control
metadata, package provenance, filenames, or manifest package versions is a test
bypass and is forbidden. The release workflow freezes the selected channel
manifest before deployment, installs its real Linux package, updates to the real
candidate package, verifies every installed binary reports the candidate
version, then explicitly downgrades to the frozen package and verifies every
binary again. Equal-version cohorts do not satisfy this gate. The hermetic local
glow-up may use one genuine cohort to prove curl install, stable/nightly asset
switching, and corporate locking; it must never claim binary upgrade or
downgrade coverage unless a second genuinely compiled cohort is supplied.

Binary GitHub releases publish host packages and the canonical host SBOM
artifact, `capsem-sbom.spdx.json`; the SBOM attestation subject list must cover
both `.pkg` and `.deb` package artifacts, and the release summary must say
`SBOM attested (SPDX 2.3, pkg + deb)`. VM asset manifests, blobs, OBOM evidence,
profile-owned records, channel manifests, and the root channel list live on
`release.capsem.org`; do not verify or publish VM `manifest.json` through the
tag release. Before recording binary metadata in the release channel, the tag
workflow preflights that the downloaded release artifacts contain
`capsem-sbom.spdx.json` and at least one installable host package (`.pkg` or
`.deb`).

Do not claim pre-updater installed clients can self-update just because the
release channel now advertises binary packages. Binaries that shipped before
the packaged binary updater must be manually bootstrapped once with the `.pkg`
or `.deb`; forward binary-update proof starts from an installed version that
already contains the updater and package apply path.

For a demo-facing macOS release, also prove the installer path users see:

```bash
just install
test -d /Applications/Capsem.app
open -a Capsem
pgrep -x capsem-service
pgrep -x capsem-tray
```

`scripts/build-pkg.sh` must install `/Applications/Capsem.app` and carry a
fallback app copy in `/usr/local/share/capsem/Capsem.app` so postinstall cannot
report success while the GUI is missing. Relaunching `Capsem.app` must ask the
running service to ensure the tray via `/companions/tray/ensure`; spawning
`capsem-tray` directly bypasses the service parent guard and is not the product
path.
## Documentation site
