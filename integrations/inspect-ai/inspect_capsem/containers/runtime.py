"""Container build, run, healthcheck, and OCI workload initialization inside a Capsem VM."""

from __future__ import annotations

import hashlib
import logging
import re
import shlex
import uuid
from pathlib import Path
from typing import TYPE_CHECKING, Any

from inspect_capsem.containers.compose import _is_bind_mount_source
from inspect_capsem.containers.controller import CapsemController
from inspect_capsem.containers.dockerfile import (
    _BUILD_DIR,
    _CA_DF_DIR,
    _CA_ENV,
    _CA_NSS_SCRIPT,
    _CA_PREP_CMD,
    _CA_READY,
    _CA_SYSTEM_SCRIPT,
    _VM_CA,
    _ca_inject_command,
    _ca_nss_command,
    _extract_ca_fingerprint,
    lower_dockerfile_heredocs,
    patch_dockerfile_for_capsem_ca,
)
from inspect_capsem.containers.image_cache import (
    _compute_image_digest,
    _format_exec_failure,
    _get_image_build_lock,
    _image_cache_dir,
    _is_image_cache_enabled,
    _max_build_context_bytes,
    _pack_path_tar_gz,
    _resolve_build_timeout,
    _save_built_image_to_cache,
    _stage_archive_to_vm_dir,
    _tar_single_file,
    _try_load_cached_image,
    pack_build_context,
)
from inspect_capsem.containers.spec import _copy_spec_with_updates

if TYPE_CHECKING:
    from inspect_capsem.containers.spec import ContainerSpec

logger = logging.getLogger(__name__)

_OCI_WORKLOAD_CONTAINER_ID = "workload"
_OCI_RUNC_EXEC_PREFIX = (
    "runc --rootless=true --root /var/tmp/capsem-container/state exec "
    "--cap CAP_SETUID --cap CAP_SETGID --cap CAP_CHOWN "
    "--cap CAP_DAC_OVERRIDE --cap CAP_FOWNER --cap CAP_KILL "
    "--cap CAP_NET_BIND_SERVICE --cap CAP_SYS_PTRACE"
)
_OCI_RUNC_EXEC = f"{_OCI_RUNC_EXEC_PREFIX} workload"
_OCI_RUNC_EXEC_ROOT = f"{_OCI_RUNC_EXEC_PREFIX} -u 0:0 workload"

_MANAGED_CONTAINER_LABEL = "inspect.capsem.managed=true"


async def _build_dockerfile_in_vm(
    controller: CapsemController, vm_id: str, spec: ContainerSpec
) -> str:
    assert spec.dockerfile is not None
    df_path = Path(spec.dockerfile)
    if not df_path.is_file():
        msg = f"Dockerfile not found: {spec.dockerfile}"
        raise FileNotFoundError(msg)
    ctx_dir = Path(spec.build_context) if spec.build_context else df_path.parent
    tar_bytes, rel_df = pack_build_context(ctx_dir, df_path)
    df_raw = df_path.read_text(encoding="utf-8")
    lowered_df = lower_dockerfile_heredocs(df_raw)
    patched_df = patch_dockerfile_for_capsem_ca(lowered_df)
    digest_meta = [rel_df]
    if spec.build_target:
        digest_meta.append(f"target={spec.build_target}")
    if spec.build_args:
        for k in sorted(spec.build_args):
            digest_meta.append(f"arg:{k}={spec.build_args[k]}")
    digest_key = "\0".join(digest_meta)
    digest_ca = _compute_image_digest(tar_bytes, digest_key, patched_df, has_ca=True)
    digest_noca = _compute_image_digest(tar_bytes, digest_key, lowered_df, has_ca=False)
    cache_enabled = _is_image_cache_enabled()
    cache_dir = _image_cache_dir()

    async with _get_image_build_lock(digest_noca):
        if cache_enabled:
            has_cached_archives = cache_dir.is_dir() and any(cache_dir.glob("*.tar.gz"))
            if has_cached_archives:
                probe_cmd = (
                    f"[ -f {_VM_CA} ] && "
                    f"{{ sha256sum {_VM_CA} 2>/dev/null | "
                    f"awk '{{print \"{_CA_READY}:\" $1}}' || echo {_CA_READY}; }} || "
                    f"echo NO_CAPSEM_CA"
                )
                probe = await controller.exec_in_vm(vm_id, probe_cmd, timeout=30)
                has_ca = _CA_READY in probe.stdout
                ca_fp = _extract_ca_fingerprint(probe.stdout)
                if ca_fp:
                    digest_ca = _compute_image_digest(
                        tar_bytes, digest_key, patched_df, has_ca=True, ca_fingerprint=ca_fp
                    )
                chosen_digest = digest_ca if has_ca else digest_noca
                chosen_tag = f"capsem-inspect-{chosen_digest}:latest"
                chosen_file = cache_dir / f"{chosen_digest}.tar.gz"
                if chosen_file.is_file() and await _try_load_cached_image(
                    controller, vm_id, chosen_tag, chosen_file
                ):
                    return chosen_tag

        df_guest = f"{_BUILD_DIR}/{rel_df}"
        await _stage_archive_to_vm_dir(controller, vm_id, tar_bytes, _BUILD_DIR)
        prep = await controller.exec_in_vm(vm_id, _CA_PREP_CMD, timeout=30)
        if prep.exit_code != 0:
            msg = (
                f"Failed preparing the Capsem CA for the Dockerfile build in VM {vm_id}: "
                f"{_format_exec_failure(prep)}"
            )
            raise RuntimeError(msg)
        has_ca = _CA_READY in prep.stdout
        ca_fp = _extract_ca_fingerprint(prep.stdout)
        if ca_fp:
            digest_ca = _compute_image_digest(
                tar_bytes, digest_key, patched_df, has_ca=True, ca_fingerprint=ca_fp
            )
        digest = digest_ca if has_ca else digest_noca
        tag = f"capsem-inspect-{digest}:latest"
        if has_ca:
            await _stage_archive_to_vm_dir(
                controller, vm_id, _tar_single_file("Dockerfile", patched_df.encode()), _CA_DF_DIR
            )
            df_guest = f"{_CA_DF_DIR}/Dockerfile"
        elif lowered_df != df_raw:
            await _stage_archive_to_vm_dir(
                controller, vm_id, _tar_single_file("Dockerfile", lowered_df.encode()), _CA_DF_DIR
            )
            df_guest = f"{_CA_DF_DIR}/Dockerfile"
        # Host networking: RUN steps on the default bridge get no DNS inside a Capsem VM.
        build_parts = [
            "docker",
            "build",
            "--network=host",
            "-t",
            shlex.quote(tag),
            "-f",
            shlex.quote(df_guest),
        ]
        if spec.build_target:
            build_parts.extend(["--target", shlex.quote(spec.build_target)])
        for k, v in spec.build_args.items():
            build_parts.extend(["--build-arg", shlex.quote(f"{k}={v}")])
        build_parts.append(_BUILD_DIR)
        build_cmd = " ".join(build_parts)
        build_timeout = _resolve_build_timeout(spec.build_timeout)
        build_res = await controller.exec_in_vm(vm_id, build_cmd, timeout=build_timeout)
        if build_res.exit_code != 0:
            msg = (
                f"Failed to build Dockerfile {spec.dockerfile} in VM {vm_id}: "
                f"{_format_exec_failure(build_res)}"
            )
            raise RuntimeError(msg)
        if cache_enabled:
            await _save_built_image_to_cache(controller, vm_id, tag, cache_dir / f"{digest}.tar.gz")
        return tag


async def _stage_bind_volumes(
    controller: CapsemController, vm_id: str, volumes: tuple[str, ...]
) -> list[str]:
    mounted: list[str] = []
    max_bytes = _max_build_context_bytes()
    for vol in volumes:
        if ":" not in vol:
            mounted.append(vol)
            continue
        src, _, rest = vol.partition(":")
        if not _is_bind_mount_source(src):
            mounted.append(vol)
            continue
        host_src = Path(src).expanduser()
        if not host_src.exists():
            mounted.append(vol)
            continue
        resolved_src = str(host_src.resolve()).encode("utf-8")
        tar_bytes = _pack_path_tar_gz(
            host_src, normalize=False, max_bytes=max_bytes, label="Bind mount"
        )
        vol_digest = hashlib.sha256(resolved_src + b"\0" + tar_bytes).hexdigest()[:12]
        stage_dir = f"/tmp/capsem-vol-{vol_digest}"
        mount_src = stage_dir if host_src.is_dir() else f"{stage_dir}/{host_src.name}"
        await _stage_archive_to_vm_dir(controller, vm_id, tar_bytes, stage_dir)
        mounted.append(f"{mount_src}:{rest}")
    return mounted


def _build_docker_run_command(
    cid_name: str,
    image: str,
    spec: ContainerSpec,
    volume_specs: list[str],
) -> str:
    # The VM's dockerd runs with bridge=none, so bridge (docker's default) is loopback only;
    # use the VM's network instead, since the VM is the isolation boundary.
    net_mode = spec.network_mode or "host"
    if net_mode in ("bridge", "default"):
        net_mode = "host"
    if net_mode.startswith("service:"):
        msg = (
            f"Unsupported Compose network_mode {net_mode!r}: "
            f"'service:<name>' is not supported directly by docker run"
        )
        raise ValueError(msg)
    parts = [
        "docker",
        "run",
        "-d",
        "--name",
        shlex.quote(cid_name),
        "--label",
        shlex.quote(_MANAGED_CONTAINER_LABEL),
    ]
    if spec.init:
        parts.append("--init")
    if spec.mem_limit:
        parts.extend(["--memory", shlex.quote(spec.mem_limit)])
    parts.extend(["--network", shlex.quote(net_mode)])
    if spec.user:
        parts.extend(["-u", shlex.quote(spec.user)])
    for k, v in spec.environment.items():
        parts.extend(["-e", shlex.quote(f"{k}={v}")])
    for p in spec.ports:
        parts.extend(["-p", shlex.quote(str(p))])
    for e in spec.expose:
        parts.extend(["--expose", shlex.quote(str(e))])

    ep_extra_args: list[str] = []
    if spec.entrypoint is not None:
        if isinstance(spec.entrypoint, tuple | list):
            if spec.entrypoint:
                parts.extend(["--entrypoint", shlex.quote(str(spec.entrypoint[0]))])
                ep_extra_args = [str(x) for x in spec.entrypoint[1:]]
        else:
            ep_str = str(spec.entrypoint)
            if ep_str == "":
                parts.extend(["--entrypoint", '""'])
            else:
                ep_tokens = shlex.split(ep_str)
                if ep_tokens:
                    parts.extend(["--entrypoint", shlex.quote(ep_tokens[0])])
                    ep_extra_args = ep_tokens[1:]
                else:
                    parts.extend(["--entrypoint", '""'])

    # Only pass -w when working_dir was explicitly set or extracted from Compose/Dockerfile,
    # so image WORKDIR is preserved otherwise; never bind-mount the VM dir over it.
    if "working_dir" in spec.explicit_fields:
        parts.extend(["-w", shlex.quote(spec.working_dir)])
    for v_spec in volume_specs:
        parts.extend(["-v", shlex.quote(v_spec)])
    parts.append(shlex.quote(image))

    if ep_extra_args:
        parts.extend(shlex.quote(a) for a in ep_extra_args)
    if spec.command is None:
        if not ep_extra_args and spec.entrypoint is None:
            parts.extend(["sleep", "infinity"])
    elif isinstance(spec.command, str):
        parts.extend(shlex.quote(arg) for arg in shlex.split(spec.command))
    else:
        parts.extend(shlex.quote(str(arg)) for arg in spec.command)
    return " ".join(parts)


_DURATION_PART_RE = re.compile(r"(\d+(?:\.\d+)?)\s*(ns|us|µs|ms|s|m|h)")


def _parse_healthcheck_duration_secs(raw: Any, *, default: int, min_secs: int = 0) -> int:
    if raw is None:
        return default
    text = str(raw).strip().lower()
    if not text:
        return default
    if re.fullmatch(r"\d+(?:\.\d+)?", text):
        total_secs = float(text)
    elif re.fullmatch(r"(?:\d+(?:\.\d+)?\s*(?:ns|us|µs|ms|s|m|h)\s*)+", text):
        total_secs = 0.0
        for m in _DURATION_PART_RE.finditer(text):
            val = float(m.group(1))
            unit = m.group(2)
            if unit == "ns":
                total_secs += val / 1_000_000_000.0
            elif unit in ("us", "µs"):
                total_secs += val / 1_000_000.0
            elif unit == "ms":
                total_secs += val / 1000.0
            elif unit == "m":
                total_secs += val * 60.0
            elif unit == "h":
                total_secs += val * 3600.0
            else:
                total_secs += val
    else:
        msg = f"Invalid healthcheck duration: {raw!r}"
        raise ValueError(msg)
    secs = 0 if total_secs <= 0 else max(1, int(total_secs))
    return max(secs, min_secs)


def _healthcheck_shell_command(hc: dict[str, Any]) -> tuple[str | None, int, int]:
    if hc.get("disable"):
        return None, 0, 0
    test = hc.get("test")
    if isinstance(test, str):
        cmd = "" if test.strip().upper() == "NONE" else test
    elif isinstance(test, list | tuple) and test:
        items = [str(x) for x in test]
        head = items[0].upper()
        if head == "NONE":
            cmd = ""
        elif head == "CMD-SHELL":
            cmd = " ".join(items[1:])
        elif head == "CMD":
            cmd = " ".join(shlex.quote(x) for x in items[1:])
        else:
            cmd = " ".join(shlex.quote(x) for x in items)
    else:
        cmd = ""
    if not cmd:
        return None, 0, 0
    retries = max(int(hc.get("retries") or 30), 1)
    interval_secs = _parse_healthcheck_duration_secs(hc.get("interval"), default=1, min_secs=1)
    return cmd, retries, interval_secs


def _healthcheck_timings(hc: dict[str, Any]) -> tuple[int, int]:
    probe_timeout_secs = _parse_healthcheck_duration_secs(hc.get("timeout"), default=30, min_secs=1)
    start_period_secs = _parse_healthcheck_duration_secs(
        hc.get("start_period"), default=0, min_secs=0
    )
    return probe_timeout_secs, start_period_secs


async def prepare_oci_workload_container(
    controller: CapsemController, vm_id: str, spec: ContainerSpec
) -> str:
    """Prepare the native `runc` `workload` container booted by `Hypervisor.create(image=...)`.

    Waits for `/var/tmp/capsem-container/workload.pid` (relaunching without the image's
    `Entrypoint` if `spec.command is None` and a non-shell `ENTRYPOINT` rejected `sleep infinity`),
    seeds `/root` with any build-time files from the unpacked OCI image's `/workspace` so the
    bind-mount does not mask them, remounts `/` read-write and mounts `/dev/shm` + executable
    `tmpfs` mounts (`/tmp`, `/scratch`, `/run`, and image `VOLUME`s) in the container's mount
    namespace, lifts the default 256 MiB / 256-pid cgroup limits on `/sys/fs/cgroup/capsem-container`,
    creates `spec.working_dir` as root, and installs the Capsem MITM CA when present.
    """
    workdir_q = shlex.quote(spec.working_dir)
    sys_script_q = shlex.quote(_CA_SYSTEM_SCRIPT)
    nss_script_q = shlex.quote(_CA_NSS_SCRIPT)
    relaunch_if_entrypoint = ""
    if spec.command is None:
        relaunch_if_entrypoint = (
            "for _ in $(seq 20); do "
            "  [ -s /var/tmp/capsem-container/workload.pid ] && break; "
            "  pgrep -f '/root/.capsem-image/launch.py' >/dev/null 2>&1 || break; "
            "  sleep 0.1; "
            "done; "
            "if [ -f /root/.capsem-image/launch.py ] && { "
            "  [ ! -s /var/tmp/capsem-container/workload.pid ] || "
            '  ! kill -0 "$(cat /var/tmp/capsem-container/workload.pid 2>/dev/null)" 2>/dev/null || '
            "  { [ -f /var/tmp/capsem-container/bundle/config.json ] && "
            '    ! python3 -c \'import json,sys; sys.exit(0 if json.load(open(sys.argv[1]))["process"]["args"]==["sleep","infinity"] else 1)\' '
            "      /var/tmp/capsem-container/bundle/config.json 2>/dev/null; }; "
            "}; then "
            '  sed -i \'s/(metadata.get("Entrypoint") or \\[\\]) + options\\["args"\\]/options["args"]/\' '
            "/root/.capsem-image/launch.py; "
            "  pkill -f '/root/.capsem-image/launch.py' 2>/dev/null || true; "
            "  runc --rootless=true --root /var/tmp/capsem-container/state delete --force workload 2>/dev/null || true; "
            "  rm -rf /var/tmp/capsem-container /root/.capsem-image/ready; "
            "  ip link del capsem0 2>/dev/null || true; "
            "  nohup python3 /root/.capsem-image/launch.py /root/.capsem-image "
            ">/var/tmp/capsem-container-relaunch.log 2>&1 </dev/null & "
            "fi; "
        )
    prep_cmd = (
        "set -e; "
        f"{relaunch_if_entrypoint}"
        "for _ in $(seq 300); do [ -s /var/tmp/capsem-container/workload.pid ] && break; "
        "sleep 0.1; done; "
        "if [ ! -s /var/tmp/capsem-container/workload.pid ]; then "
        "  echo 'OCI workload container pid not ready after 30s' >&2; exit 1; "
        "fi; "
        "pid=$(cat /var/tmp/capsem-container/workload.pid); "
        "if [ -d /var/tmp/capsem-container/bundle/rootfs/workspace ]; then "
        "  tar -C /var/tmp/capsem-container/bundle/rootfs/workspace --exclude='./.capsem-image' -cf - . 2>/dev/null "
        "  | tar -C /root -xkf - 2>/dev/null || true; "
        "fi; "
        'nsenter -t "$pid" -m sh -c \''
        "mount -o remount,rw / / && "
        "mkdir -p /dev/shm && "
        "(mountpoint -q /dev/shm || mount -t tmpfs -o rw,nosuid,nodev,mode=1777,size=512m tmpfs /dev/shm) && "
        "for d in /tmp /scratch /run; do "
        '  mountpoint -q "$d" && mount -o remount,rw,exec "$d" "$d" || true; '
        "done && "
        'awk \'"\'"\'$3 == "tmpfs" && $2 !~ "^/(proc|dev|sys)$" && $2 !~ "/\\\\.capsem-image$" {print $2}\'"\'"\' /proc/mounts '
        '| while read -r mp; do mount -o remount,rw,exec "$mp" "$mp" 2>/dev/null || true; done\'; '
        "for f in memory.max memory.swap.max pids.max cpu.max; do "
        '  [ -w "/sys/fs/cgroup/capsem-container/$f" ] && '
        '  echo max > "/sys/fs/cgroup/capsem-container/$f" 2>/dev/null || true; '
        "done; "
        "chmod a+rx /root 2>/dev/null || true; "
        f"{_OCI_RUNC_EXEC_ROOT} mkdir -p {workdir_q}; "
        f"{_OCI_RUNC_EXEC_ROOT} chmod a+rx {workdir_q} 2>/dev/null || true; "
        f"if [ -f {_VM_CA} ]; then "
        f"  {_OCI_RUNC_EXEC_ROOT} sh -c {sys_script_q} < {_VM_CA}; "
        f"  {_OCI_RUNC_EXEC} sh -c {nss_script_q} >/dev/null 2>&1 || true; "
        "fi"
    )
    res = await controller.exec_in_vm(vm_id, prep_cmd, timeout=90)
    if res.exit_code != 0:
        msg = f"Failed preparing OCI workload container in VM {vm_id}: {_format_exec_failure(res)}"
        raise RuntimeError(msg)
    return _OCI_WORKLOAD_CONTAINER_ID


async def start_container_for_init(
    controller: CapsemController,
    vm_id: str,
    spec: ContainerSpec,
) -> str:
    ready = await controller.exec_in_vm(
        vm_id,
        "for _ in $(seq 60); do docker info >/dev/null 2>&1 && exit 0; sleep 1; done; docker info",
        timeout=90,
    )
    if ready.exit_code != 0:
        msg = f"Docker daemon in VM {vm_id} not ready after 60s: {_format_exec_failure(ready)}"
        raise RuntimeError(msg)

    image = (
        await _build_dockerfile_in_vm(controller, vm_id, spec) if spec.dockerfile else spec.image
    )
    volume_specs = (
        await _stage_bind_volumes(controller, vm_id, spec.volumes) if spec.volumes else []
    )
    has_ca = await controller.exec_in_vm(vm_id, f"[ -f {_VM_CA} ] && echo {_CA_READY}", timeout=30)
    inject_ca = _CA_READY in has_ca.stdout
    if inject_ca:
        spec = _copy_spec_with_updates(spec, {"environment": {**_CA_ENV, **spec.environment}})

    cid_name = f"inspect-{uuid.uuid4().hex[:8]}"
    run_cmd = _build_docker_run_command(cid_name, image, spec, volume_specs)
    run_timeout = max(120, _resolve_build_timeout(spec.build_timeout))
    res = await controller.exec_in_vm(vm_id, run_cmd, timeout=run_timeout)
    if res.exit_code != 0:
        msg = f"Failed to start container in VM {vm_id}: {_format_exec_failure(res)}"
        raise RuntimeError(msg)
    lines = res.stdout.strip().splitlines()
    cid = (lines[-1].strip() if lines else "") or cid_name

    try:
        if inject_ca:
            ca_res = await controller.exec_in_vm(vm_id, _ca_inject_command(cid), timeout=120)
            if ca_res.exit_code != 0:
                msg = (
                    f"Failed installing the Capsem CA in container {cid} in VM {vm_id}: "
                    f"{_format_exec_failure(ca_res)}"
                )
                raise RuntimeError(msg)
            nss_res = await controller.exec_in_vm(
                vm_id, _ca_nss_command(cid, user=spec.user), timeout=60
            )
            if nss_res.exit_code != 0:
                logger.warning(
                    "Could not add the Capsem CA to the NSS DB in container %s; Chromium there "
                    "will reject HTTPS: %s",
                    cid,
                    _format_exec_failure(nss_res),
                )

        if spec.healthcheck:
            hc_cmd, retries, interval_secs = _healthcheck_shell_command(spec.healthcheck)
            if hc_cmd:
                probe_timeout_secs, start_period_secs = _healthcheck_timings(spec.healthcheck)
                probe_exec = (
                    f"timeout {probe_timeout_secs} "
                    f"docker exec {shlex.quote(cid)} sh -c {shlex.quote(hc_cmd)}"
                )
                start_prefix = (
                    f"_hc_deadline=$(($(date +%s) + {start_period_secs})); "
                    f"while [ $(date +%s) -lt $_hc_deadline ]; do "
                    f"{probe_exec} >/dev/null 2>&1 && exit 0; "
                    f"sleep {interval_secs}; "
                    f"done; "
                    if start_period_secs > 0
                    else ""
                )
                wait_cmd = (
                    f"{start_prefix}"
                    f"for _ in $(seq {retries}); do "
                    f"{probe_exec} >/dev/null 2>&1 && exit 0; "
                    f"sleep {interval_secs}; "
                    f"done; "
                    f"{probe_exec}"
                )
                per_probe_budget = interval_secs + probe_timeout_secs
                overall_timeout = max(start_period_secs + retries * per_probe_budget + 30, 60)
                hc_res = await controller.exec_in_vm(vm_id, wait_cmd, timeout=overall_timeout)
                if hc_res.exit_code != 0:
                    msg = (
                        f"Container {cid} in VM {vm_id} failed healthcheck: "
                        f"{_format_exec_failure(hc_res)}"
                    )
                    raise RuntimeError(msg)
    except BaseException:
        try:
            await controller.exec_in_vm(vm_id, f"docker rm -f {shlex.quote(cid)}", timeout=30)
        except Exception:
            logger.debug("Failed removing container %s after failed init", cid, exc_info=True)
        raise

    return cid
