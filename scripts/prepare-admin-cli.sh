#!/bin/bash
# prepare-admin-cli.sh -- Build the packaged capsem-admin wrapper payload.
#
# Usage: prepare-admin-cli.sh <output_bin_dir>
#
# Produces:
#   <output_bin_dir>/capsem-admin
#   <output_bin_dir>/capsem-admin-python/
#
# The wrapper is intentionally relocatable. In a build tree it loads the
# sibling capsem-admin-python directory; in installed packages it loads the
# platform share directory copied by build-pkg.sh/repack-deb.sh.
set -euo pipefail

OUT_DIR="${1:?usage: prepare-admin-cli.sh <output_bin_dir>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ADMIN_PYTHON_DIR="$OUT_DIR/capsem-admin-python"
WRAPPER="$OUT_DIR/capsem-admin"

if ! command -v uv >/dev/null 2>&1; then
    echo "ERROR: uv is required to prepare capsem-admin package payload" >&2
    exit 1
fi

mkdir -p "$OUT_DIR"
rm -rf "$ADMIN_PYTHON_DIR"
mkdir -p "$ADMIN_PYTHON_DIR"

PYTHON_FOR_PACKAGE="$(
    cd "$REPO_ROOT"
    uv run python -c 'import sys; print(sys.executable)'
)"
PYTHON_PACKAGE_VERSION="$("$PYTHON_FOR_PACKAGE" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')"

(
    cd "$REPO_ROOT"
    uv pip install --python "$PYTHON_FOR_PACKAGE" --target "$ADMIN_PYTHON_DIR" "$REPO_ROOT"
)
printf '%s\n' "$PYTHON_PACKAGE_VERSION" > "$ADMIN_PYTHON_DIR/.capsem-python-version"

cat > "$WRAPPER" <<'SH'
#!/bin/sh
set -eu

self_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ -n "${CAPSEM_ADMIN_PYTHON:-}" ]; then
    python_bin="$CAPSEM_ADMIN_PYTHON"
else
    python_bin=""
    for candidate_python in python3.14 python3.13 python3.12 python3.11 python3; do
        if command -v "$candidate_python" >/dev/null 2>&1 \
            && "$candidate_python" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
            python_bin="$candidate_python"
            break
        fi
    done
fi
if [ -z "$python_bin" ]; then
    echo "capsem-admin: Python 3.11 or newer is required" >&2
    exit 127
fi

for candidate in \
    "${CAPSEM_ADMIN_PYTHONPATH:-}" \
    "$self_dir/capsem-admin-python" \
    "$self_dir/../admin-python" \
    "/usr/local/share/capsem/admin-python" \
    "/usr/share/capsem/admin-python"
do
    if [ -n "$candidate" ] && [ -d "$candidate/capsem/admin" ]; then
        if [ -f "$candidate/.capsem-python-version" ]; then
            required_version=$(cat "$candidate/.capsem-python-version")
            actual_version=$("$python_bin" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
            if [ "$actual_version" != "$required_version" ]; then
                echo "capsem-admin: packaged Python payload requires Python $required_version; got $actual_version" >&2
                echo "capsem-admin: set CAPSEM_ADMIN_PYTHON to a matching interpreter or install the PyPI package for this host" >&2
                exit 127
            fi
        fi
        export PYTHONPATH="$candidate${PYTHONPATH:+:$PYTHONPATH}"
        exec "$python_bin" -m capsem.admin.cli "$@"
    fi
done

echo "capsem-admin: packaged Python payload not found" >&2
exit 127
SH
chmod 755 "$WRAPPER"

CAPSEM_ADMIN_PYTHON="$PYTHON_FOR_PACKAGE" "$WRAPPER" --version >/dev/null

echo "Prepared capsem-admin wrapper at $WRAPPER"
echo "Prepared capsem-admin Python payload at $ADMIN_PYTHON_DIR"
