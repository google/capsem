#!/bin/bash
# Sourced only by the existing disposable Tart owner in explicit paired mode.

capsem_pair_field() {
    python3 -m capsem_builder.release.tools.release_macos_apply field --root "$SHARE" "$@"
}

capsem_pair_http() {
    local command="$1"
    shift
    local transport=(--socket "$CAPSEM_HOME/run/service.sock")
    if [ "$command" = request ]; then
        transport=(--gateway-run-dir "$CAPSEM_HOME/run")
    fi
    python3 -m helpers.installed_apply_probe "$command" \
        "${transport[@]}" \
        --timeout "$(capsem_pair_field --field timeout)" \
        --maximum "$(capsem_pair_field --field maximum)" "$@"
}

capsem_pair_verify() {
    local phase="$1" version package metadata_url binary_names
    version=$(capsem_pair_field --phase "$phase" --field version)
    package=$(capsem_pair_field --phase "$phase" --field package)
    metadata_url=$(capsem_pair_field --phase "$phase" --field metadata_manifest_url)
    local extra=()
    if [ "$(capsem_pair_field --field scope)" = public_predecessor_upgrade ]; then
        if [ "$phase" = before ]; then
            extra=(--legacy-profile-predecessor)
        else
            metadata_url="$MANIFEST_URL"
        fi
    fi
    binary_names=$(python3 -m capsem_builder.release.tools.release_macos_apply binaries \
        --root "$SHARE" --phase "$phase")
    test -n "$binary_names"
    BINARIES=()
    while IFS= read -r binary; do
        BINARIES+=("$binary")
    done <<< "$binary_names"
    VERSION="$version" verify_binary_cohort
    "$CAPSEM" version > "$SHARE/binary-apply-$phase-version.txt"
    /usr/sbin/pkgutil --pkg-info com.capsem.pkg | grep -Fx "version: $version"
    test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
        /Applications/Capsem.app/Contents/Info.plist)" = "$version"
    python3 "$VERIFY" --capsem "$CAPSEM" --manifest-url "$MANIFEST_URL" \
        --metadata-manifest-url "$metadata_url" --channel "$CHANNEL" \
        --package-version "$version" --artifact "$package" --platform macos \
        --architecture arm64 --evidence-out "$SHARE/binary-apply-$phase-installed.json" "${extra[@]}"
}

capsem_public_state() {
    python3 -m capsem_builder.release.tools.release_macos_public_upgrade "$1" \
        --root "$SHARE" --home "$CAPSEM_HOME" --source "$MANIFEST_URL"
}

capsem_prove_binary_apply() {
    local before_package before_manifest old_service_pid new_service_pid
    export PYTHONPATH="$SHARE"
    python3 -m capsem_builder.release.tools.release_macos_apply check --root "$SHARE"
    before_package=$(capsem_pair_field --phase before --field package)
    before_manifest=$(capsem_pair_field --phase before --field manifest)
    # Existing fixture-owned app control prevents even the first startup from
    # polling a different generation while native hydration is in progress.
    mkdir -p "$CAPSEM_HOME"
    printf '%s\n' '[settings."app.auto_update"]' 'value = false' \
        'modified = "Tart binary Apply fixture"' > "$CAPSEM_HOME/settings.toml"
    bash "$INSTALL_USER_REQUEST" write admin
    bash "$INSTALL_MANIFEST_REQUEST" write "$before_manifest" "$MANIFEST_URL"
    sudo /usr/sbin/installer -pkg "$before_package" -target /
    clear_install_user_request
    capsem_finish_install_hydration
    capsem_pair_verify before
    capsem_pair_http disable-automatic --output "$SHARE/binary-apply-automatic-state.json"
    launchctl kickstart -k "gui/$(id -u)/com.capsem.service"
    capsem_wait_for_service
    local scope
    scope=$(capsem_pair_field --field scope)
    if [ "$scope" = public_predecessor_upgrade ]; then
        # Ordinary original CLI selection of EXACT public BEFORE bytes. This
        # deliberate fixture configuration is not package metadata rewriting.
        "$CAPSEM" update --assets --manifest "$MANIFEST_URL"
        capsem_public_state capture
    fi
    old_service_pid=$(cat "$CAPSEM_HOME/run/service.pid")
    case "$old_service_pid" in ''|*[!0-9]*) return 1 ;; esac
    test "$old_service_pid" -gt 0
    kill -0 "$old_service_pid"
    audit_line > "$SHARE/binary-apply-audit-line"
    python3 "$TRANSITION_SUPPORT" promote "$SHARE/original-manifest.json" "$REMOTE_MANIFEST"
    python3 "$TRANSITION_SUPPORT" assert-url "$MANIFEST_URL" "$SHARE/original-manifest.json"
    if [ "$scope" = public_predecessor_upgrade ]; then
        # SAME candidate bytes will be consumed by NEW package postinstall/CLI.
        bash "$INSTALL_MANIFEST_REQUEST" write "$SHARE/original-manifest.json" "$MANIFEST_URL"
    fi
    capsem_pair_http request --output "$SHARE/binary-apply-request.json" \
        --source "$MANIFEST_URL" --digest "$(capsem_pair_field --phase after --field manifest_sha256)" \
        --source-commit "$(capsem_pair_field --phase after --field source_commit)" \
        --audit "$CAPSEM_HOME/logs/update.log" --after-line-file "$SHARE/binary-apply-audit-line"
    # No kickstart or package retry here: the original transaction must retire
    # its own old service and activate its exact candidate under launchd.
    capsem_wait_for_service "$old_service_pid"
    new_service_pid=$(cat "$CAPSEM_HOME/run/service.pid")
    test "$new_service_pid" != "$old_service_pid"
    if kill -0 "$old_service_pid" 2>/dev/null; then
        echo "ERROR: old launchd service survived original Apply" >&2
        return 1
    fi
    if [ "$scope" = public_predecessor_upgrade ]; then
        capsem_wait_for_assets "$SHARE/public-upgrade-runtime-status.json"
        capsem_public_state preserved
        observe_update_transition public_predecessor_upgrade activated \
            "$(capsem_pair_field --phase after --field manifest_sha256)" \
            "$(cat "$SHARE/binary-apply-audit-line")" "$SHARE/binary-apply-transition.json"
    else
        observe_update_transition binary_only activated \
            "$(capsem_pair_field --phase after --field manifest_sha256)" \
            "$(cat "$SHARE/binary-apply-audit-line")" "$SHARE/binary-apply-transition.json"
    fi
    capsem_pair_verify after
    python3 -m capsem_builder.release.tools.release_macos_apply retirement --root "$SHARE" \
        --old-pid "$old_service_pid" --new-pid "$new_service_pid"
    python3 -m capsem_builder.release.tools.release_macos_apply finish --root "$SHARE" \
        --source "$MANIFEST_URL"
}
