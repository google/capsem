# Capsem execution-lock helper.
#
# Source this file, then call `acquire_exec_lock <path>` to open fd 3 on
# the given lockfile and take a non-blocking flock(2). When the host has no
# `flock` binary (GitHub macOS runners), a small Python fcntl holder process
# keeps the same advisory lock until the calling shell exits. Exits the
# current shell with a clear message if another agent already holds the lock.
#
# Call sites (justfile):
#   just dev / shell / run / bench / release / ... ->
#       $HOME/.capsem/run/execution.lock  (shared with the dev service)
#   just test / smoke ->
#       <repo>/target/capsem-test-execution.lock  (outside $CAPSEM_HOME so
#       it survives the `rm -rf $CAPSEM_HOME` wipe; same-file path across
#       invocations, so the advisory lock actually collides and blocks
#       concurrent test runs)

_release_python_exec_lock() {
    if [[ -n "${CAPSEM_EXEC_LOCK_PID:-}" ]]; then
        kill "$CAPSEM_EXEC_LOCK_PID" 2>/dev/null || true
        wait "$CAPSEM_EXEC_LOCK_PID" 2>/dev/null || true
    fi
    if [[ -n "${CAPSEM_EXEC_LOCK_STATUS_FILE:-}" ]]; then
        rm -f "$CAPSEM_EXEC_LOCK_STATUS_FILE"
    fi
}

_acquire_python_exec_lock() {
    local lock_file="$1"
    local status_file
    status_file="$(mktemp "${TMPDIR:-/tmp}/capsem-exec-lock.XXXXXX")"

    python3 - "$lock_file" "$status_file" <<'PY' &
import errno
import fcntl
import os
import signal
import sys
import time

lock_file = sys.argv[1]
status_file = sys.argv[2]


def write_status(status):
    with open(status_file, "w", encoding="utf-8") as handle:
        handle.write(status)


try:
    fd = os.open(lock_file, os.O_RDWR | os.O_CREAT, 0o666)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError as exc:
        if exc.errno in (errno.EACCES, errno.EAGAIN):
            write_status("LOCKED")
            raise SystemExit(75)
        raise
    write_status("HELD")
    parent_pid = os.getppid()

    def stop(_signum, _frame):
        raise SystemExit(0)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while True:
        if os.getppid() != parent_pid:
            raise SystemExit(0)
        time.sleep(1)
except SystemExit:
    raise
except Exception as exc:
    write_status("ERROR")
    print(f"failed to acquire capsem execution lock with python: {exc}", file=sys.stderr)
    raise SystemExit(1)
PY
    local lock_pid=$!
    local status
    for _ in {1..250}; do
        if [[ -s "$status_file" ]]; then
            status="$(cat "$status_file")"
            case "$status" in
                HELD)
                    CAPSEM_EXEC_LOCK_PID="$lock_pid"
                    CAPSEM_EXEC_LOCK_STATUS_FILE="$status_file"
                    trap _release_python_exec_lock EXIT
                    return 0
                    ;;
                LOCKED)
                    wait "$lock_pid" 2>/dev/null || true
                    rm -f "$status_file"
                    return 75
                    ;;
                *)
                    wait "$lock_pid" 2>/dev/null || true
                    rm -f "$status_file"
                    return 1
                    ;;
            esac
        fi
        sleep 0.02
    done

    kill "$lock_pid" 2>/dev/null || true
    wait "$lock_pid" 2>/dev/null || true
    rm -f "$status_file"
    echo "timed out while acquiring capsem execution lock ($lock_file)" >&2
    return 1
}

acquire_exec_lock() {
    local lock_file="$1"
    mkdir -p "$(dirname "$lock_file")"

    if [[ "${CAPSEM_EXEC_LOCK_FORCE_PYTHON:-0}" != "1" ]] && command -v flock >/dev/null 2>&1; then
        exec 3>"$lock_file"
        flock -n 3 || {
            echo "another agent holds the capsem execution lock ($lock_file); try again later" >&2
            exit 1
        }
        return 0
    fi

    if ! command -v python3 >/dev/null 2>&1; then
        echo "python3 is required to acquire the capsem execution lock when flock is unavailable" >&2
        exit 1
    fi

    _acquire_python_exec_lock "$lock_file"
    case "$?" in
        0) return 0 ;;
        75)
            echo "another agent holds the capsem execution lock ($lock_file); try again later" >&2
            exit 1
            ;;
        *) exit 1 ;;
    esac
}
