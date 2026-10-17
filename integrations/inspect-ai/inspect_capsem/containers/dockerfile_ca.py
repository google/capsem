"""Pure Dockerfile support from Pierre Tholoniat's bb61fc82d integration."""

from __future__ import annotations

from .dockerfile_scan import _dockerfile_instructions

_CA_READY = "CAPSEM_CA_READY"

_IMAGE_CA = "/usr/local/share/ca-certificates/capsem-ca.crt"

_IMAGE_CA_BUNDLE = "/usr/local/share/capsem/ca-bundle.crt"

_CA_ENV = {
    "SSL_CERT_FILE": _IMAGE_CA_BUNDLE,
    "REQUESTS_CA_BUNDLE": _IMAGE_CA_BUNDLE,
    "PIP_CERT": _IMAGE_CA_BUNDLE,
    "CURL_CA_BUNDLE": _IMAGE_CA_BUNDLE,
    "GIT_SSL_CAINFO": _IMAGE_CA_BUNDLE,
    "NODE_EXTRA_CA_CERTS": _IMAGE_CA,
    "UV_NATIVE_TLS": "1",
}

_CA_STAGE_LINES = (
    f"COPY .capsem-ca.crt {_IMAGE_CA}",
    f"COPY .capsem-ca-bundle.crt {_IMAGE_CA_BUNDLE}",
    "ENV " + " ".join(f"{k}={v}" for k, v in _CA_ENV.items()),
)

_CA_UPDATE_LINE = (
    "RUN command -v update-ca-certificates >/dev/null 2>&1 "
    '&& { [ "$(id -u)" = 0 ] && update-ca-certificates >/dev/null 2>&1 '
    "|| sudo -n update-ca-certificates >/dev/null 2>&1; } || true"
)


def _extract_ca_fingerprint(stdout: str) -> str:
    """Return the hex SHA-256 fingerprint emitted after `CAPSEM_CA_READY:`, if present."""
    for line in stdout.splitlines():
        s = line.strip()
        if s.startswith(f"{_CA_READY}:"):
            return s.split(":", 1)[1].strip()
    return ""


def _stage_runs_shell(rest: list[tuple[str, str, list[str]]]) -> bool:
    """Whether the stage starting at `rest` has a shell-form RUN (so /bin/sh exists)."""
    for keyword, text, _ in rest:
        if keyword == "FROM":
            return False
        if keyword == "RUN":
            args = [a for a in text.split()[1:] if not a.startswith("--")]
            if args and not args[0].startswith("["):
                return True
    return False


def _stage_has_any_run(rest: list[tuple[str, str, list[str]]]) -> bool:
    """Whether the stage starting at `rest` has any RUN instruction (shell or exec form)."""
    for keyword, _, _ in rest:
        if keyword == "FROM":
            return False
        if keyword == "RUN":
            return True
    return False


def is_root_user_spec(user: str | None) -> bool:
    """Return True when `user` is unset, empty, or resolves to root (`root`, `0`, `0:0`, `root:root`)."""
    if user is None:
        return True
    head = user.strip().split(":", 1)[0].strip().lower()
    return head in ("", "root", "0")


def patch_dockerfile_for_capsem_ca(text: str) -> str:
    """Add the Capsem MITM CA to every stage that can make TLS calls during the build.

    Build stages need it as much as the final one (they fetch too). Stages `FROM scratch`
    are skipped (no RUN can execute there), as are stages built from an earlier stage,
    which inherit the CA files. If an earlier stage had no shell-form RUN, a derived stage
    that *does* run a shell still emits `update-ca-certificates` (wrapping in `USER root`
    and restoring the inherited user if the parent stage ended as non-root). And if a stage
    switches back to `USER root` / `USER 0` before a `RUN` (shell or exec form),
    `update-ca-certificates` is emitted after the `USER` switch so the system store is
    updated as root.
    """
    instructions = _dockerfile_instructions(text.splitlines())
    out: list[str] = []
    stages: set[str] = set()
    updated_stages: set[str] = set()
    root_updated_stages: set[str] = set()
    stage_end_user: dict[str, str] = {}
    current_alias: str | None = None
    current_has_ca_files = False
    current_runs_shell = False
    current_updated_as_root = False
    current_user = ""
    for idx, (keyword, joined, raw) in enumerate(instructions):
        out.extend(raw)
        if keyword == "USER":
            parts = joined.split(None, 1)
            user_spec = parts[1].strip() if len(parts) > 1 else ""
            current_user = user_spec
            if current_alias:
                stage_end_user[current_alias] = current_user
            if (
                user_spec
                and is_root_user_spec(user_spec)
                and current_has_ca_files
                and not current_updated_as_root
                and (current_runs_shell or _stage_has_any_run(instructions[idx + 1 :]))
            ):
                if not out or out[-1] != _CA_UPDATE_LINE:
                    out.append(_CA_UPDATE_LINE)
                current_updated_as_root = True
                if current_alias:
                    updated_stages.add(current_alias)
                    root_updated_stages.add(current_alias)
            continue
        if keyword != "FROM":
            continue
        args = [a for a in joined.split()[1:] if not a.startswith("--")]
        base = args[0].lower() if args else "scratch"
        alias = args[2].lower() if len(args) >= 3 and args[1].lower() == "as" else None
        current_alias = alias
        current_runs_shell = _stage_runs_shell(instructions[idx + 1 :])
        has_ca_files = base in stages
        has_ca_update = base in updated_stages
        current_updated_as_root = base in root_updated_stages
        current_user = stage_end_user.get(base, "")
        if base != "scratch":
            if not has_ca_files:
                out.extend(_CA_STAGE_LINES)
                has_ca_files = True
            if has_ca_files and not has_ca_update and current_runs_shell:
                if current_user and not is_root_user_spec(current_user):
                    out.append("USER root")
                    out.append(_CA_UPDATE_LINE)
                    out.append(f"USER {current_user}")
                    has_ca_update = True
                    current_updated_as_root = True
                else:
                    out.append(_CA_UPDATE_LINE)
                    has_ca_update = True
                    if current_user and is_root_user_spec(current_user):
                        current_updated_as_root = True
        current_has_ca_files = has_ca_files
        if alias:
            stage_end_user[alias] = current_user
            if has_ca_files:
                stages.add(alias)
            if has_ca_update:
                updated_stages.add(alias)
            if current_updated_as_root:
                root_updated_stages.add(alias)
    return "\n".join(out) + "\n"
