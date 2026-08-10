"""Strict configuration for the host-kernel sandbox and release egress edge."""

from __future__ import annotations

from pydantic import model_validator

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
        if not any(
            self.linux_args[index : index + 3] == ("--dev-bind", "/dev", "/dev")
            for index in range(max(0, len(self.linux_args) - 2))
        ):
            raise ValueError(
                "linux_args must preserve usable host devices with --dev-bind /dev /dev"
            )
        for required in ("--die-with-parent", "--new-session"):
            if required not in self.linux_args:
                raise ValueError(f"linux_args must contain {required}")
        return self
