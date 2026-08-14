# shellcheck shell=bash
# Deliberately no shebang: this file is sourced, never executed. The directive
# tells ShellCheck which dialect to assume.
#
# Capsem execution-lock helper.
#
# Source this file, then use one of two operations:
#
# - `acquire_exec_lock <path>` holds a non-blocking lock on fd 3 for the
#   remainder of the current shell.
# - `run_with_exec_lock <path> <command> ...` holds a blocking lock for one
#   command.
#
# util-linux `flock` is optional. macOS runners use Python's standard-library
# fcntl fallback, which applies the same kernel advisory lock to the same file.
#
# Call sites (justfile):
#   just dev / shell / run / bench / release / ... ->
#       $HOME/.capsem/run/execution.lock  (shared with the dev service)
#   just test / smoke ->
#       <repo>/target/capsem-test-execution.lock  (outside $CAPSEM_HOME so
#       it survives the `rm -rf $CAPSEM_HOME` wipe; same-file path across
#       invocations, so flock(2) actually collides and blocks concurrent
#       test runs)

_capsem_try_lock_fd() {
    if command -v flock >/dev/null 2>&1; then
        flock -n 3
    else
        python3 -c 'import fcntl; fcntl.flock(3, fcntl.LOCK_EX | fcntl.LOCK_NB)'
    fi
}

acquire_exec_lock() {
    local lock_file="$1"
    mkdir -p "$(dirname "$lock_file")"
    exec 3>"$lock_file"
    _capsem_try_lock_fd || {
        echo "another agent holds the capsem execution lock ($lock_file); try again later" >&2
        exit 1
    }
}

run_with_exec_lock() {
    local lock_file="$1"
    shift
    if [[ "$#" -eq 0 ]]; then
        echo "run_with_exec_lock requires a command" >&2
        return 2
    fi
    mkdir -p "$(dirname "$lock_file")"
    if command -v flock >/dev/null 2>&1; then
        flock "$lock_file" "$@"
    else
        python3 -c '
import fcntl
import os
import sys

lock = open(sys.argv[1], "a")
fcntl.flock(lock, fcntl.LOCK_EX)
os.set_inheritable(lock.fileno(), True)
os.execvp(sys.argv[2], sys.argv[2:])
' "$lock_file" "$@"
    fi
}
