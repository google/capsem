"""The workload's syscall filter, observed from inside a real workload.

`capsem run --image` launches the production launcher, so the workload runs
under the filter capsem-core resolves (deny by default, moby's allowlist,
no namespace creation, no mount). The probe is the image's own shell: a
process entered from outside with nsenter would not carry the filter. An
image declaring the Xpra surface gets exactly the namespaces Chromium's
sandbox creates, and nothing else.

vsock, keyctl and netfilter netlink need a probe program the fixture image
does not have; the filter's content is proven by capsem-core's
`container::seccomp` tests and the full in-workload probe arrives with the
capsem-debug image.
"""

import subprocess

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import command, environment, service

__all__ = ["service"]

pytestmark = pytest.mark.integration

PROBE = (
    "for probe in 'unshare -U true' 'unshare -n true' 'mount -t tmpfs none /mnt' 'nsenter -t 1 -m true'; do "
    '  if $probe 2>/dev/null; then echo "ALLOWED $probe"; else echo "DENIED $probe"; fi; '
    "done; "
    'echo "ORDINARY $(echo works)"'
)


def test_the_workload_cannot_create_namespaces_or_mount(service, tmp_path):
    with registry(tmp_path) as (reference, certificate, _):
        result = subprocess.run(
            [*command(service, reference, certificate), "/bin/sh", "-c", PROBE],
            env=environment(service),
            capture_output=True,
            timeout=120,
            check=False,
        )
    (tmp_path / "probe.stdout").write_bytes(result.stdout)
    (tmp_path / "probe.stderr").write_bytes(result.stderr)
    output = result.stdout.decode(errors="replace")
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    assert "ORDINARY works" in output, output
    assert "ALLOWED" not in output, output
    assert output.count("DENIED") == 4, output


# Chromium's namespace sandbox, as the filter of each surface sees it. Every
# case runs in a forked child, which exits with the call's errno (0 when it
# succeeded), so no case changes the namespaces of the next. The refused
# cases all ask for a user namespace too: the kernel itself grants an
# unprivileged process every namespace beside a new user namespace, so only
# the filter can be what refuses them. perl, in the Debian iperf3 image, makes
# the raw calls: busybox `unshare` has no cgroup or time namespace, and
# clone() is how Chromium asks.
CLONE_NEWNS = 0x0002_0000
CLONE_NEWCGROUP = 0x0200_0000
CLONE_NEWUTS = 0x0400_0000
CLONE_NEWIPC = 0x0800_0000
CLONE_NEWUSER = 0x1000_0000
CLONE_NEWPID = 0x2000_0000
CLONE_NEWNET = 0x4000_0000
CLONE_NEWTIME = 0x0000_0080
CHROMIUM = CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET
EPERM = 1

NAMESPACE_PROBE = r"""
use POSIX ();
my %nr = (aarch64 => [220, 97, 51], x86_64 => [56, 272, 161]);
my ($clone, $unshare, $chroot) = @{$nr{(POSIX::uname())[4]}};
sub child { my $pid = fork(); if ($pid == 0) { POSIX::_exit($_[0]->()) } waitpid($pid, 0); $? >> 8 }
sub errno { $_[0] < 0 ? $! + 0 : 0 }
for my $case (@ARGV) {
  my ($call, $flags) = split /=/, $case;
  $flags = hex $flags;
  my $errno = child(sub {
    if ($call eq "unshare") { return errno(syscall($unshare, $flags)) }
    my $root = "/";
    if ($call eq "chroot") { my $e = errno(syscall($unshare, $flags)); return $e || errno(syscall($chroot, $root)) }
    my $pid = syscall($clone, $flags | 17, 0, 0, 0, 0);
    POSIX::_exit(0) if $pid == 0;
    return errno($pid) if $pid < 0;
    waitpid($pid, 0);
    return 0;
  });
  print "$case $errno\n";
}
"""

# (call, flags) -> outcome on the (xpra, terminal) surface.
NAMESPACE_CASES = {
    # What Chromium does: user, PID and network namespaces, by clone and by
    # unshare, then chroot inside the new user namespace.
    ("clone", CHROMIUM): ("allowed", "refused"),
    ("unshare", CHROMIUM): ("allowed", "refused"),
    ("unshare", CLONE_NEWUSER): ("allowed", "refused"),
    ("chroot", CLONE_NEWUSER): ("allowed", "refused"),
    # What stays refused on every surface.
    ("clone", CLONE_NEWUSER | CLONE_NEWNS): ("refused", "refused"),
    ("clone", CLONE_NEWUSER | CLONE_NEWIPC): ("refused", "refused"),
    ("unshare", CLONE_NEWUSER | CLONE_NEWNS): ("refused", "refused"),
    ("unshare", CLONE_NEWUSER | CLONE_NEWIPC): ("refused", "refused"),
    ("unshare", CLONE_NEWUSER | CLONE_NEWUTS): ("refused", "refused"),
    ("unshare", CLONE_NEWUSER | CLONE_NEWCGROUP): ("refused", "refused"),
    ("unshare", CLONE_NEWUSER | CLONE_NEWTIME): ("refused", "refused"),
    ("unshare", CHROMIUM | CLONE_NEWNS): ("refused", "refused"),
}


@pytest.mark.parametrize(
    ("labels", "column"),
    [
        ({"org.capsem.surface": "xpra", "org.capsem.surface.port": "14500"}, 0),
        ({}, 1),
    ],
    ids=["xpra", "terminal"],
)
def test_only_the_xpra_surface_allows_chromiums_namespace_sandbox(
    service, tmp_path, labels, column
):
    config = {
        "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
        "Cmd": ["true"],
        "Labels": labels,
    }
    cases = [f"{call}={flags:#x}" for call, flags in NAMESPACE_CASES]
    with registry(tmp_path, image="iperf3", image_config=config) as (reference, certificate, _):
        result = subprocess.run(
            [*command(service, reference, certificate), "perl", "-e", NAMESPACE_PROBE, *cases],
            env=environment(service),
            capture_output=True,
            timeout=120,
            check=False,
        )
    (tmp_path / "probe.stdout").write_bytes(result.stdout)
    (tmp_path / "probe.stderr").write_bytes(result.stderr)
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    observed = dict(line.rsplit(" ", 1) for line in result.stdout.decode().splitlines())
    expected = {
        case: "0" if outcomes[column] == "allowed" else str(EPERM)
        for case, outcomes in zip(cases, NAMESPACE_CASES.values(), strict=True)
    }
    assert observed == expected, observed
