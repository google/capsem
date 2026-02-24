#!/bin/bash
# Capsem VM tool verification
# Run with: capsem-test

PASS=0; FAIL=0
DIR="/root/tests"
mkdir -p "$DIR"

t() {
    local name="$1"; shift
    if "$@" > "$DIR/$name.out" 2>&1; then
        echo "  PASS  $name"; ((PASS++))
    else
        echo "  FAIL  $name (exit $?)"; ((FAIL++))
    fi
}

echo "=== Permission Model ==="

# /root must be writable
echo "write test" > /root/tests/perm_write.txt
if [ -f /root/tests/perm_write.txt ]; then
    echo "  PASS  root_writable"; ((PASS++))
else
    echo "  FAIL  root_writable"; ((FAIL++))
fi

# rootfs must be read-only
if touch /usr/.ro_test 2>/dev/null; then
    rm -f /usr/.ro_test
    echo "  FAIL  rootfs_readonly (write succeeded!)"; ((FAIL++))
else
    echo "  PASS  rootfs_readonly"; ((PASS++))
fi

# All top-level dirs except writable mounts must reject writes
WRITABLE="/root /tmp /run /dev /proc /sys"
for d in /*/; do
    d="${d%/}"
    case " $WRITABLE " in
        *" $d "*) continue ;;
    esac
    if touch "$d/.ro_test" 2>/dev/null; then
        rm -f "$d/.ro_test"
        echo "  FAIL  ro:${d} (write succeeded!)"; ((FAIL++))
    else
        echo "  PASS  ro:${d}"; ((PASS++))
    fi
done

echo ""
echo "=== Unix Utilities ==="
t df          df -h
t ps          ps aux
t free        free -m
t lsof        lsof -v
t find        find / -maxdepth 1 -type d
t grep        grep --version
t sed         sed --version
t awk         awk --version
t less        less --version
t file        file /bin/bash
t tar         tar --version
t strace      strace -V
t lsblk       lsblk
t mount-list  mount
t id          id
t hostname    hostname
t uname       uname -a
t uptime      uptime
t dmesg       dmesg | head -5
t vim         vim --version
t du          du -sh /root

echo ""
echo "=== Dev Runtimes ==="
t python3     python3 --version
t pip3        pip3 --version
t node        node --version
t npm         npm --version
t git         git --version

echo ""
echo "=== AI CLIs ==="
t claude      command -v claude
t gemini      command -v gemini
t codex       command -v codex

echo ""
echo "=== File Write Workflow ==="
echo "capsem write test $(date)" > "$DIR/write_test.txt"
if [ -f "$DIR/write_test.txt" ]; then
    echo "  PASS  write_file"; ((PASS++))
else
    echo "  FAIL  write_file"; ((FAIL++))
fi

echo ""
echo "=== Python Workflow ==="
python3 -c "
import json, sys, os
data = {'test': 'ok', 'pid': os.getpid(), 'python': sys.version}
with open('/root/tests/python_workflow.json', 'w') as f:
    json.dump(data, f, indent=2)
print('wrote python_workflow.json')
" && { echo "  PASS  python_workflow"; ((PASS++)); } \
  || { echo "  FAIL  python_workflow"; ((FAIL++)); }

echo ""
echo "Results: $PASS passed, $FAIL failed"
echo "Test outputs in: $DIR/"
