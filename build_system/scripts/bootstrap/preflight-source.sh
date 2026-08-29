#!/usr/bin/env bash
# Ephemeral-model and guest-binary source preflight checks.

# --------------------------------------------------------------------------
# Check: capsem-init does not allow state to persist between VM sessions.
# Invariants:
#   (1) scratch disk is always formatted unconditionally at boot (no ext4 detection skip)
#   (2) overlay upperdir is always on tmpfs, never on the scratch disk
# See web/docs/ephemeral_model.md for the incident that motivated these checks.
# --------------------------------------------------------------------------
check_ephemeral_model() {
    echo ""
    echo "== Ephemeral VM Model =="

    local init="$ROOT_DIR/guest/artifacts/capsem-init"

    if [[ ! -f "$init" ]]; then
        fail "capsem-init not found at $init"
        return
    fi

    # FAIL: conditional mke2fs skip on scratch disk (block mode must always format).
    # The VirtioFS ext4 loopback check (blkid + mke2fs for /mnt/shared/system/rootfs.img)
    # is expected and safe -- it only formats on first boot of the loopback image.
    # We only flag if the BLOCK mode scratch disk (/dev/vdb) conditionally skips formatting.
    if grep -B5 'mke2fs.*scratch\|mke2fs.*vdb' "$init" | grep -qE 'blkid.*ext4'; then
        fail "capsem-init conditionally skips mke2fs on scratch disk -- would persist across reboots"
    else
        pass "capsem-init: scratch disk always formatted (or not used)"
    fi

    # FAIL: scratch disk used as overlay upper layer
    if grep -qE 'UPPER=.*scratch|upperdir[=[:space:]].*scratch' "$init"; then
        fail "capsem-init uses scratch disk as overlayfs upper -- all rootfs writes would persist"
    else
        pass "capsem-init: scratch disk not used as overlay upper"
    fi

    # PASS: mke2fs must be present (scratch disk formatted at boot)
    if grep -q 'mke2fs' "$init"; then
        pass "capsem-init: mke2fs present (scratch disk formatted at every boot)"
    else
        fail "capsem-init: mke2fs missing -- scratch disk never formatted"
    fi

    # PASS: tmpfs used for overlay upper directory
    if grep -qE 'mount -t tmpfs tmpfs /mnt/b' "$init"; then
        pass "capsem-init: tmpfs used for overlay upper layer"
    else
        fail "capsem-init: tmpfs overlay upper not found -- writes may persist"
    fi

    # PASS: tmpfs mount failure must abort boot (no silent degraded mode)
    if grep -qE 'exit 1' "$init" && grep -A3 'mount -t tmpfs tmpfs /mnt/b' "$init" | grep -q 'exit 1'; then
        pass "capsem-init: tmpfs mount failure aborts boot (no silent degraded fallback)"
    else
        fail "capsem-init: tmpfs mount failure does not abort boot -- VM may start with wrong upper layer"
    fi

    # VirtioFS mode checks
    if grep -q 'mount -t virtiofs capsem' "$init"; then
        pass "capsem-init: VirtioFS overlay path present"
    else
        fail "capsem-init: VirtioFS overlay path missing"
    fi

    if grep -A5 'mount -t virtiofs capsem' "$init" | grep -q 'exit 1'; then
        pass "capsem-init: VirtioFS mount failure aborts boot"
    else
        fail "capsem-init: VirtioFS mount failure does not abort boot"
    fi
}

# --------------------------------------------------------------------------
# Check: every [[bin]] in capsem-agent is referenced in Dockerfile + justfile
# Source of truth: crates/capsem-agent/Cargo.toml [[bin]] entries.
# --------------------------------------------------------------------------
check_guest_binaries() {
    echo ""
    echo "== Guest Binaries =="

    local cargo_toml="$ROOT_DIR/crates/capsem-agent/Cargo.toml"
    local dockerfile="$ROOT_DIR/config/docker/Dockerfile.rootfs.j2"
    local justfile="$ROOT_DIR/justfile"

    if [[ ! -f "$cargo_toml" ]]; then
        fail "capsem-agent Cargo.toml not found at $cargo_toml"
        return
    fi

    # Extract [[bin]] name values from Cargo.toml
    local binaries
    binaries=$(grep -A1 '^\[\[bin\]\]' "$cargo_toml" | grep '^name' | sed 's/.*= *"\(.*\)"/\1/')

    if [[ -z "$binaries" ]]; then
        fail "no [[bin]] entries found in $cargo_toml"
        return
    fi

    for bin in $binaries; do
        # Guest binaries are injected via initrd repack, not baked into rootfs.
        # Check justfile _pack-initrd references the binary.
        if grep -q "$bin" "$justfile"; then
            pass "justfile _pack-initrd: $bin"
        else
            fail "justfile missing $bin in _pack-initrd"
        fi
    done
}
