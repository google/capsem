"""Strict configuration for the host-kernel sandbox and release egress edge."""

from __future__ import annotations

from ipaddress import ip_address

from pydantic import PositiveFloat, PositiveInt, model_validator

from .configschema import Strict


class SandboxConfig(Strict):
    """What the host-kernel sandbox is allowed to say.

    macOS renders Seatbelt rules; Linux enters a Bubblewrap network namespace.
    Every command, argument, and rule comes from configuration so a security
    boundary cannot acquire a literal nobody knows where to change.
    """

    command: str
    linux_command: str
    linux_args: tuple[str, ...]
    linux_device_mount: str
    linux_probe_loopback_host: str
    linux_probe_egress_host: str
    linux_probe_egress_port: PositiveInt
    linux_probe_timeout_seconds: PositiveFloat
    linux_hosted_failure_marker: str
    linux_hosted_repair_command: tuple[str, ...]
    linux_hosted_userns_sysctl: str
    linux_hosted_userns_required_value: int
    linux_hosted_userns_repair_value: int
    profile_name: str
    egress_metadata_variable: str
    egress_socket_template: str
    egress_metadata_template: str
    egress_start_timeout: float
    egress_stop_timeout: float
    egress_max_message_bytes: int
    network_reason: str
    socket_reason: str
    sockets: tuple[str, ...]
    local_socket_prefixes: tuple[str, ...]
    local_socket_regexes: tuple[str, ...]
    local_binds: bool
    loopback: tuple[str, ...]
    log_command: str
    report_predicate: str
    report_style: str
    report_log_name: str
    report_summary_suffix: str
    report_pid_suffix: str
    report_stop_timeout: float

    @model_validator(mode="after")
    def _linux_network_namespace_is_an_enforcement_boundary(self) -> SandboxConfig:
        """A configurable wrapper must not be configurable into doing nothing."""
        if self.linux_args.count("--unshare-net") != 1:
            raise ValueError("linux_args must contain exactly one --unshare-net")
        if not any(
            self.linux_args[index : index + 3] == ("--bind", "/", "/")
            for index in range(max(0, len(self.linux_args) - 2))
        ):
            raise ValueError("linux_args must preserve the host filesystem with --bind / /")
        device_mount = self.linux_device_mount
        if not any(
            self.linux_args[index : index + 3] == ("--dev-bind", device_mount, device_mount)
            for index in range(max(0, len(self.linux_args) - 2))
        ):
            raise ValueError(
                "linux_args must preserve its configured host device mount with "
                f"--dev-bind {device_mount} {device_mount}"
            )
        for required in ("--die-with-parent", "--new-session"):
            if required not in self.linux_args:
                raise ValueError(f"linux_args must contain {required}")
        if not ip_address(self.linux_probe_loopback_host).is_loopback:
            raise ValueError("linux_probe_loopback_host must be a loopback address")
        egress = ip_address(self.linux_probe_egress_host)
        if egress.is_loopback or egress.is_unspecified or egress.is_multicast:
            raise ValueError("linux_probe_egress_host must name an external unicast address")
        if self.linux_probe_egress_port > 65535:
            raise ValueError("linux_probe_egress_port must fit a TCP port")
        if self.linux_hosted_repair_command != ("sudo", "sysctl", "-w"):
            raise ValueError("the hosted repair must remain the narrow sudo sysctl command")
        if (
            self.linux_hosted_userns_sysctl != "kernel.apparmor_restrict_unprivileged_userns"
            or self.linux_hosted_userns_required_value != 1
            or self.linux_hosted_userns_repair_value != 0
        ):
            raise ValueError("the hosted repair may only lift Ubuntu's userns AppArmor switch")
        return self
