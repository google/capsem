#!/bin/sh
# Run Claude Desktop as the one application of an Xpra session, and exit when
# it exits.
#
# Order matters, because Claude reads each prerequisite once at startup:
#   1. a private D-Bus session bus, shared by the keyring and Claude;
#   2. the login keyring, unlocked before Claude asks the Secret Service for
#      its stored login (otherwise Claude warns that logins will not persist);
#   3. the runtime's CA bundle, imported into the user's NSS database, which
#      Chromium trusts instead of the system bundle;
#   4. Xpra, which starts its own X display and then Claude in it.
# The container exits with Claude's exit status.
#
# Chromium keeps its sandbox. The image runs with no_new_privs, so the setuid
# chrome-sandbox helper is inert and Chromium uses its user-namespace sandbox
# instead. That needs clone() with CLONE_NEWUSER|CLONE_NEWPID|CLONE_NEWNET and
# unshare(), which the runtime's default syscall filter refuses; the Xpra
# surface's filter must allow both, or Claude aborts at startup.
# Never add --no-sandbox here.
set -eu

# Must match the org.capsem.surface.port label in the Dockerfile.
port=14500

export HOME="${HOME:-/home/capsem}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-$(id -u)}"
install -d -m 0700 "$XDG_RUNTIME_DIR"
status="$XDG_RUNTIME_DIR/claude-desktop.status"

# Xpra's child: run Claude and record how it ended, because Xpra exits 0 when
# its last child exits, whatever that child's status was. The bus started
# before the display existed, so the services it activates for Claude (the
# GTK file-chooser portal) learn the display here, or they cannot open it.
if [ "${1:-}" = --child ]; then
    dbus-update-activation-environment DISPLAY XAUTHORITY
    rc=0
    # Chromium picks its secret store from the desktop name, and Xpra's is
    # unknown to it, so it would fall back to a plaintext store and Claude's
    # login would not persist. Name the Secret Service explicitly.
    claude-desktop --password-store=gnome-libsecret || rc=$?
    echo "$rc" >"$status"
    exit "$rc"
fi

if [ -z "${CAPSEM_GUI_BUS:-}" ]; then
    CAPSEM_GUI_BUS=1 exec dbus-run-session -- "$0" "$@"
fi
unset CAPSEM_GUI_BUS

# --- 2. login keyring --------------------------------------------------------
# No display manager logs this user in, so nothing hands PAM a password to
# unlock the keyring with. The first start makes a random one and keeps it
# beside the keyring (mode 0600), so a named session unlocks the same keyring
# every time; an ephemeral session gets a fresh one with its fresh home.
state="$HOME/.config/capsem"
secret="$state/keyring-unlock"
install -d -m 0700 "$state" "$HOME/.local/share/keyrings"
if [ ! -s "$secret" ]; then
    (umask 077 && od -An -N32 -tx1 /dev/urandom | tr -d ' \n' >"$secret")
fi
chmod 0600 "$secret"
gnome-keyring-daemon --login <"$secret" >/dev/null
gnome-keyring-daemon --start --components=secrets >/dev/null

# --- 3. CA trust in NSS ------------------------------------------------------
# Every certificate of the bundle becomes a trusted TLS issuer (C,,) under a
# nickname derived from its PEM, so a restart re-imports nothing, and a
# certificate that left the bundle (a rotated Capsem CA) leaves the database.
bundle="${SSL_CERT_FILE:-/etc/ssl/certs/ca-certificates.crt}"
nssdb="$HOME/.pki/nssdb"
stamp="$nssdb/capsem-bundle.sha256"
install -d -m 0700 "$HOME/.pki" "$nssdb"
if [ ! -f "$nssdb/cert9.db" ]; then
    certutil -N --empty-password -d "sql:$nssdb"
fi
digest="$(sha256sum <"$bundle" | cut -d' ' -f1)"
if [ "$(cat "$stamp" 2>/dev/null)" != "$digest" ]; then
    split="$XDG_RUNTIME_DIR/ca-bundle"
    rm -rf "$split" && mkdir -m 0700 "$split"
    awk -v dir="$split" '/-----BEGIN CERTIFICATE-----/ { n++ } n { print > (dir "/" n ".pem") }' "$bundle"
    held="$(certutil -L -d "sql:$nssdb" | awk '$1 ~ /^capsem-/ { print $1 }')"
    wanted=""
    for pem in "$split"/*.pem; do
        [ -e "$pem" ] || continue
        name="capsem-$(sha256sum <"$pem" | cut -c1-16)"
        wanted="$wanted $name "
        case " $held " in *" $name "*) continue ;; esac
        certutil -A -d "sql:$nssdb" -n "$name" -t C,, -i "$pem"
    done
    for name in $held; do
        case "$wanted" in *" $name "*) ;; *) certutil -D -d "sql:$nssdb" -n "$name" ;; esac
    done
    rm -rf "$split"
    echo "$digest" >"$stamp"
fi

# --- 4. Xpra -----------------------------------------------------------------
# The surface speaks only Xpra's websocket protocol, on loopback, with no
# authentication: the Capsem gateway owns authentication, and this port is
# reachable only through its authenticated preview route. The gateway also
# serves the pinned xpra-html5 client, so Xpra serves no HTML (html=off).
#
# One application, no desktop: no tray, no window-manager features, nothing
# forwarded but the window, input and clipboard. Xpra uses the D-Bus session
# above rather than launching its own, and it does not pass the session's
# address on to its children by itself, so it is handed over explicitly:
# without it Claude cannot reach the unlocked keyring.
rm -f "$status"
xpra start :100 \
    --daemon=no \
    --bind=none \
    --bind-ws="127.0.0.1:$port,auth=none" \
    --html=off \
    --ssh-upgrade=no \
    --mmap=no \
    --socket-dirs="$XDG_RUNTIME_DIR/xpra" \
    --log-dir="$XDG_RUNTIME_DIR" \
    --systemd-run=no \
    --dbus-launch=no \
    --dbus-control=no \
    --start-env="DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS" \
    --mdns=no \
    --audio=no \
    --webcam=no \
    --printing=no \
    --file-transfer=no \
    --open-files=no \
    --open-url=no \
    --notifications=no \
    --system-tray=no \
    --start-child="$0 --child" \
    --exit-with-children=yes \
    --terminate-children=yes

# Claude's status, or failure when Xpra ended without Claude having exited.
exit "$(cat "$status" 2>/dev/null || echo 1)"
