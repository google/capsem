# Sprint T16: Guest Validation
## Goal
Verify guest is correctly configured after boot (network, services, filesystem, env)

## Files
- tests/capsem-guest/conftest.py
- tests/capsem-guest/test_guest_network.py
- tests/capsem-guest/test_guest_services.py
- tests/capsem-guest/test_guest_filesystem.py
- tests/capsem-guest/test_guest_env.py

## Tasks
- [x] test_guest_network: ip link shows lo+dummy0, iptables REDIRECT to 10443, ss shows net-proxy listening, resolv.conf shows localhost, ping 8.8.8.8 fails
- [x] test_guest_services: pgrep capsem-pty-agent running, pgrep capsem-net-proxy running, pgrep dnsmasq running
- [x] test_guest_filesystem: mount shows ro rootfs, overlay tmpfs upper, /capsem/workspace exists, touch /bin/test fails
- [x] test_guest_env: HOME=/root, TERM set, PATH includes expected dirs, LD_PRELOAD empty
- [x] All use shared_vm fixture, exec via service API
- [x] Marked guest

## Verification
pytest tests/capsem-guest/ -m guest passes

## Depends On
T14
