# Sprint: Standalone Installer

## Context

The installation infrastructure is 90% built across two prior sprints (native-installer, install-lifecycle) but has gaps preventing a seamless end-to-end experience. This sprint **consolidates and closes** the remaining work from both, plus adds proper package bundling.

**Goal:** A user downloads the package (.pkg on macOS, .deb on Linux) or runs `just install`, and everything is ready -- all 6 binaries installed, service + gateway + tray running, assets available, setup completed. No manual steps.

**Assumption:** capsem-app compile errors (boot_vm/send_boot_config API changes) will be fixed before this sprint starts.

## Supersedes

This sprint replaces the unchecked items from:
- `sprints/native-installer/` -- remaining test gate, e2e verification, WB4 binary swap (deferred)
- `sprints/install-lifecycle/` -- SS0-SS5 (clean baseline, graceful stop, health check, tray verification)

**Housekeeping:** Move both to `sprints/done/`:
```
mv sprints/native-installer sprints/done/
mv sprints/install-lifecycle sprints/done/
```

---

## SS0: Binary Parity -- All 6 Binaries Everywhere

Every code path that enumerates capsem binaries must list all 6: `capsem, capsem-service, capsem-process, capsem-mcp, capsem-gateway, capsem-tray`.

**Files to modify:**

1. `crates/capsem/src/uninstall.rs:53-55` -- Add `"capsem-gateway"` and `"capsem-tray"` to `CAPSEM_BINARIES`

2. `crates/capsem/src/uninstall.rs:40-48` -- Add pkill calls for `capsem-gateway` and `capsem-tray`

3. `tests/capsem-install/conftest.py:24` -- Update `BINARIES` to all 6

4. `tests/capsem-install/conftest.py:47-66` -- Update `_kill_service()` to also kill gateway and tray

5. `tests/capsem-install/conftest.py:74` -- Fix docstring "all 4 binaries" -> "all 6 binaries"

**Verify:** `cargo test -p capsem` passes; grep confirms no stale "4 binaries" references

---

## SS1: Graceful Stop + Health Check + Auto-Setup in `just install`

Make the dev install path safe, verified, and fully automated.

**Files to modify -- `justfile` install recipe (line ~451):**

1. **Before** `simulate-install.sh` -- stop existing service:
   ```bash
   if [ -f "$HOME/.capsem/bin/capsem" ]; then
       echo "=== Stopping existing service ==="
       "$HOME/.capsem/bin/capsem" service uninstall 2>/dev/null || true
       pkill -9 -x capsem-service 2>/dev/null || true
       pkill -9 -x capsem-gateway 2>/dev/null || true
       pkill -9 -x capsem-tray 2>/dev/null || true
       pkill -9 -x capsem-process 2>/dev/null || true
       sleep 0.5
       rm -f "$HOME/.capsem/run/service.sock"
   fi
   ```

2. **After** `capsem service install` -- health check:
   ```bash
   echo "=== Verifying service health ==="
   HEALTHY=false
   for i in $(seq 1 30); do
       if [ -S "$HOME/.capsem/run/service.sock" ] && \
          curl -s --unix-socket "$HOME/.capsem/run/service.sock" --max-time 2 http://localhost/list >/dev/null 2>&1; then
           echo "Service is responding."
           HEALTHY=true
           break
       fi
       sleep 0.5
   done
   if [ "$HEALTHY" != "true" ]; then
       echo "WARNING: Service not responding after 15s."
       echo "Check: ~/Library/Logs/capsem/service.log"
   fi
   ```

3. **After** health check -- auto-setup on first install:
   ```bash
   if [ ! -f "$HOME/.capsem/setup-state.json" ]; then
       echo "=== Running initial setup ==="
       "$HOME/.capsem/bin/capsem" setup --non-interactive --accept-detected
   fi
   ```

**Verify:** `just install` from clean state: stops old service -> copies 6 binaries -> codesigns -> registers service -> health check passes -> setup runs

---

## SS2: CLI Auto-Setup on First Use

When a user runs any sandbox command without prior `capsem setup`, auto-run non-interactive setup. This covers the package install path where the user installs the .pkg/.deb and then runs `capsem shell` from terminal.

**Files to modify:**

1. `crates/capsem/src/main.rs` (~line 548, before `UdsClient::new`) -- Add:
   ```rust
   // Auto-setup on first use (service/setup/misc commands already handled above)
   let setup_done = paths::capsem_home()
       .map(|d| d.join("setup-state.json").exists())
       .unwrap_or(false);
   if !setup_done {
       eprintln!("First run detected. Running initial setup...");
       eprintln!("(Run `capsem setup` to reconfigure later)\n");
       setup::run_setup(setup::SetupOptions {
           non_interactive: true,
           preset: None,
           force: false,
           accept_detected: true,
           corp_config: None,
       }).await?;
   }
   ```

**Verify:** `rm ~/.capsem/setup-state.json && capsem list` triggers setup automatically

---

## SS3: macOS .pkg Installer

Replace the .dmg with a .pkg that bundles the .app + all 6 companion binaries + assets + a postinstall script.

**New files to create:**

1. `scripts/build-pkg.sh` -- Assembles the .pkg after Tauri builds the .app:
   - Takes signed .app, companion binaries, assets, entitlements as input
   - Uses `pkgbuild` to create a component package with a payload dir containing:
     - `Applications/Capsem.app` (the Tauri GUI)
     - `usr/local/share/capsem/bin/{capsem,capsem-service,capsem-process,capsem-mcp,capsem-gateway,capsem-tray}`
     - `usr/local/share/capsem/assets/{manifest.json,vmlinuz,initrd.img}`
     - `usr/local/share/capsem/entitlements.plist`
   - Uses `productbuild --distribution` to wrap with license/welcome/distribution XML
   - Signs with `productsign` if identity available

2. `scripts/pkg-scripts/postinstall` -- Runs as the installing user after .pkg install:
   ```bash
   #!/bin/bash
   CAPSEM_DIR="$HOME/.capsem"
   mkdir -p "$CAPSEM_DIR/bin" "$CAPSEM_DIR/assets" "$CAPSEM_DIR/run"
   PKG_SHARE="/usr/local/share/capsem"
   # Copy binaries
   for bin in capsem capsem-service capsem-process capsem-mcp capsem-gateway capsem-tray; do
       cp "$PKG_SHARE/bin/$bin" "$CAPSEM_DIR/bin/$bin"
       chmod 755 "$CAPSEM_DIR/bin/$bin"
   done
   # Codesign (required for Virtualization.framework)
   for bin in "$CAPSEM_DIR/bin"/capsem*; do
       codesign --sign - --entitlements "$PKG_SHARE/entitlements.plist" --force "$bin"
   done
   # Copy assets
   cp -r "$PKG_SHARE/assets/"* "$CAPSEM_DIR/assets/"
   # PATH setup
   # Register service + run setup
   "$CAPSEM_DIR/bin/capsem" service install
   "$CAPSEM_DIR/bin/capsem" setup --non-interactive --accept-detected
   ```

3. `scripts/pkg-distribution.xml` -- productbuild distribution descriptor (title, license, domains)

**Files to modify:**

4. `.github/workflows/release.yaml` build-app-macos job -- After Tauri build:
   - Build all companion binaries: `cargo build --release -p capsem -p capsem-service -p capsem-process -p capsem-mcp -p capsem-gateway -p capsem-tray`
   - Codesign companion binaries with Developer ID
   - Run `scripts/build-pkg.sh`
   - Notarize .pkg (`xcrun notarytool submit`)
   - Upload .pkg as release artifact (replace .dmg)

**Verify:** Build .pkg locally, install it on clean macOS, verify: 6 binaries in `~/.capsem/bin/`, service running, setup-state.json exists

---

## SS4: Linux .deb with Companion Binaries

Add all 6 companion binaries and a postinst script to the Linux .deb package. Tauri's deb bundler doesn't support postinst scripts natively, so we repack after Tauri builds.

**New files to create:**

1. `scripts/repack-deb.sh` -- Repacks the Tauri .deb:
   - Extract with `dpkg-deb -R`
   - Copy companion binaries into `usr/bin/`
   - Create `DEBIAN/postinst` from `scripts/deb-postinst.sh`
   - Repack with `dpkg-deb -b`

2. `scripts/deb-postinst.sh` -- Post-install for .deb:
   ```bash
   #!/bin/bash
   # Run as installing user on first login / manual trigger
   # Creates ~/.capsem layout, registers systemd unit, runs setup
   ```

**Files to modify:**

3. `.github/workflows/release.yaml` build-app-linux job -- After Tauri build:
   - Build companion binaries: `cargo build --release -p capsem -p capsem-service ...`
   - Run `scripts/repack-deb.sh`
   - Re-validate with `dpkg-deb --info`

**Verify:** Install repacked .deb in Docker, verify companion binaries present, service registered

---

## SS5: Test Hardening & Verification

Close the testing gaps inherited from native-installer and install-lifecycle.

**Actions:**
1. `RUSTFLAGS="-D warnings" cargo check --workspace` -- zero warnings
2. `cargo test --workspace` -- all unit tests pass
3. `cargo test -p capsem` -- verify capsem crate specifically
4. `just install` on macOS -- full end-to-end with stop/health/setup
5. `just test-install` -- Docker e2e suite (all 14 test files)
6. Verify test_lifecycle.py passes end-to-end in Docker
7. Verify test_reinstall.py proves install is not silently a no-op
8. Verify error path tests produce actionable error messages (not stack traces)
9. Manual macOS: auto-launch, service install/uninstall, setup wizard, LaunchAgent, tray icon

**Files that may need updates** based on test results:
- Test files referencing "4 binaries" in assertions/comments
- `test_lifecycle.py` if auto-setup changes expected output
- `test_uninstall.py` to verify all 6 binaries removed

---

## SS6: Acceptance Gate

- [ ] `cargo check --workspace` -- zero errors
- [ ] `RUSTFLAGS="-D warnings" cargo check --workspace` -- zero warnings
- [ ] `cargo test --workspace` -- all tests pass
- [ ] **Dev path:** `just install` on macOS -- stop -> install 6 binaries -> codesign -> register -> health check -> auto-setup
- [ ] After `just install`: `capsem service status` Installed + Running
- [ ] After `just install`: `capsem list` responds
- [ ] After `just install`: tray icon visible in menu bar
- [ ] `capsem uninstall --yes` removes all 6 binaries + service + ~/.capsem/
- [ ] **macOS package:** .pkg install -> 6 binaries in ~/.capsem/bin/ -> service registered -> setup done
- [ ] **Linux package:** .deb install -> companion binaries -> service registered -> setup done
- [ ] `just test-install` -- Docker e2e passes
- [ ] CHANGELOG.md updated

## Dependency Graph

```
SS0 (binary parity) ---+
SS1 (just install)  ---+--> SS5 (test hardening) --> SS6 (acceptance)
SS2 (CLI auto-setup) --+
SS3 (macOS .pkg)    ---+
SS4 (Linux .deb)    ---+
```

SS0-SS4 are independent and can be worked in parallel. SS5/SS6 validate everything.

## Critical Files

| File | SS | Change |
|------|-----|--------|
| `crates/capsem/src/uninstall.rs` | SS0 | Add gateway/tray to binary + kill lists |
| `tests/capsem-install/conftest.py` | SS0 | Update BINARIES to 6, fix _kill_service |
| `justfile` (install recipe) | SS1 | Pre-stop, health check, auto-setup |
| `crates/capsem/src/main.rs` | SS2 | Auto-setup before first service command |
| `scripts/build-pkg.sh` | SS3 | New: build macOS .pkg from Tauri output |
| `scripts/pkg-scripts/postinstall` | SS3 | New: .pkg post-install script |
| `scripts/pkg-distribution.xml` | SS3 | New: productbuild distribution |
| `.github/workflows/release.yaml` | SS3+SS4 | Build companions, .pkg, repack .deb |
| `scripts/repack-deb.sh` | SS4 | New: inject binaries + postinst into .deb |
| `scripts/deb-postinst.sh` | SS4 | New: .deb post-install script |

## Deferred

- **Self-update binary swap (WB6):** `self-replace` crate in Cargo.toml but needs release infra for binary artifacts
- **Public install.sh for CLI-only:** needs standalone binary tarballs on GitHub releases
- **UI/frontend rewrite:** separate workstream
