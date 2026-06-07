# Capsem execution-lock helper.
#
# Source this file, then call `acquire_exec_lock <path>` to open fd 3 on
# the given lockfile and take a non-blocking flock(2). Exits the current
# shell with a clear message if another agent already holds the lock.
#
# Call sites (justfile):
#   just dev / shell / run / bench / release / ... ->
#       $HOME/.capsem/run/execution.lock  (shared with the dev service)
#   just test / smoke ->
#       <repo>/target/capsem-test-execution.lock  (outside $CAPSEM_HOME so
#       it survives the `rm -rf $CAPSEM_HOME` wipe; same-file path across
#       invocations, so flock(2) actually collides and blocks concurrent
#       test runs)

acquire_exec_lock() {
    local lock_file="$1"
    mkdir -p "$(dirname "$lock_file")"
    exec 3>"$lock_file"
    flock -n 3 || {
        echo "another agent holds the capsem execution lock ($lock_file); try again later" >&2
        exit 1
    }
}
