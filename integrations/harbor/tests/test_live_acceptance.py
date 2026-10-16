"""Live acceptance tests for `harbor-capsem` across 5 Harbor benchmarks and endpoint modes."""

from __future__ import annotations

import contextlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from collections.abc import AsyncGenerator
from pathlib import Path
from typing import Any

import pytest
from capsem import Hypervisor
from capsem.trace import load_rows_tables
from harbor.job import Job
from harbor.models.agent.name import AgentName
from harbor.models.job.config import JobConfig
from harbor.models.task.config import EnvironmentConfig as TaskEnvConfig
from harbor.models.trial.config import AgentConfig, EnvironmentConfig, TaskConfig
from harbor.models.trial.paths import TrialPaths
from harbor_capsem import CapsemEnvironment, events_to_atif

_WARN = ("DeprecationWarning", "UserWarning", "ResourceWarning")
pytestmark = [
    pytest.mark.skipif(os.environ.get("CAPSEM_LIVE_SELF_CHECK") != "1", reason="Live check off"),
    *(pytest.mark.filterwarnings(f"ignore::{w}") for w in _WARN),
    pytest.mark.filterwarnings("ignore::pytest.PytestUnraisableExceptionWarning"),
]
_CRED_REF_RE = re.compile(r"^credential:blake3:[0-9a-f]{64}$")
_EXEC_CRED_RE = re.compile(r"GEMINI_API_KEY=(credential:blake3:[0-9a-f]{64})")
_HUB_CACHE = Path.home() / ".cache" / "inspect_harbor" / "tasks" / "hub"
_TOK_KEYS = ("prompt_tokens", "completion_tokens")
_EXTRA_BENCH = (("camel_1361", "camel-ai/1361/*"), ("quix_mst", "quixbugs/python-*_tree/*"))
_EXTRA_OPT = (("usaco_118", "usaco/118/*"), ("vector_ea", "thetalab/vector-edit-gym-ea-011/*"))


def _find_cached_task(rel_glob: str = "terminal-bench/fix-git/*") -> Path:
    if matches := sorted(p for p in _HUB_CACHE.glob(rel_glob) if (p / "task.toml").is_file()):
        return matches[-1]
    pytest.skip(f"Cached Harbor task not found under {_HUB_CACHE}/{rel_glob}")


def _gcloud(*args: str) -> str:
    proc = subprocess.run(["gcloud", *args], capture_output=True, text=True, check=False)
    if proc.returncode == 0 and (val := proc.stdout.strip()) and val != "(unset)":
        return val
    pytest.skip(f"gcloud {' '.join(args)} unconfigured")


def _resolve_runtime() -> tuple[Path, Path, Path, Path]:
    auth = os.environ.get("CAPSEM_CACHE_AUTHORITY", "").strip()
    adirs = [Path(auth).expanduser() / s for s in ("cache/target", "target")] if auth else []
    tdirs = [*adirs, *(p / "cache/target" for p in Path(__file__).resolve().parents)]
    acands = [Path(os.environ["CAPSEM_ASSETS_DIR"])] if "CAPSEM_ASSETS_DIR" in os.environ else []
    acands.extend([*(d / "assets" for d in tdirs), Path.home() / ".capsem/assets"])
    if not (adir := next((c for c in acands if c.is_dir()), None)):
        pytest.skip("Missing Capsem 0.7 VM assets directory")
    out: list[Path] = [adir / "x86_64" if (adir / "x86_64").is_dir() else adir]
    b_env = (("CAPSEM_BIN_DIR", ""), ("CARGO_TARGET_DIR", "debug"))
    bdirs = [Path(os.environ[k]) / s for k, s in b_env if os.environ.get(k)]
    for name in ("capsem-service", "capsem-process", "capsem-gateway"):
        bc = [*(d / name for d in bdirs), *(d / "cargo/debug" / name for d in tdirs)]
        if not (b := next((c for c in [*bc, Path(shutil.which(name) or "")] if c.is_file()), None)):
            pytest.skip(f"Could not locate {name} binary for live acceptance test")
        out.append(b)
    return out[0], out[1], out[2], out[3]


@pytest.fixture(scope="module")
def live_capsem_env() -> Any:
    proj = os.environ.get("VERTEXAI_PROJECT") or _gcloud("config", "get-value", "project")
    capsem_home = Path(tempfile.mkdtemp(prefix="capsem-harbor-home-", dir="/var/tmp"))
    assets_dir, svc_bin, proc_bin, gw_bin = _resolve_runtime()
    run_dir, creds_path = capsem_home / "run", capsem_home / "credential-store.json"
    run_dir.mkdir(parents=True, exist_ok=True)
    if (default_creds := Path.home() / ".capsem/credentials/credential-store.json").is_file():
        shutil.copy2(default_creds, creds_path)
    if (oci_cache := Path.home() / ".capsem/cache").is_dir():
        (capsem_home / "cache").symlink_to(oci_cache)
    srcs = '"ghcr.io", "docker.io", "127.0.0.1:5055", "127.0.0.1:5055/inspect-capsem/build"'
    admit = '"ghcr.io", "docker.io", "127.0.0.1:5055/inspect-capsem/build"'
    m_expr = 'http.host == "127.0.0.1" || http.host == "localhost" || ip.value == "127.0.0.1"'
    rule = f'[profiles.rules.allow_local_proxy]\nname="local"\naction="allow"\nmatch=\'{m_expr}\'\n'
    cfg_txt = f"[images]\nsources=[{srcs}]\nadmit=[{admit}]\n{rule}"
    (capsem_home / "settings.toml").write_text(cfg_txt, encoding="utf-8")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        port = int(s.getsockname()[1])
    prefix, uds_path = f"hb-live-{os.getpid()}", run_dir / "capsem-service.sock"
    svc_env = {**os.environ, "CAPSEM_RUN_DIR": str(run_dir), "CAPSEM_HOME": str(capsem_home)}
    svc_env["CAPSEM_CREDENTIAL_STORE_PATH"] = str(creds_path)
    cmd_str = (
        f"{svc_bin} --uds-path {uds_path} --assets-dir {assets_dir} --process-binary {proc_bin} "
        f"--gateway-port {port} --gateway-binary {gw_bin} --parent-pid {os.getpid()} --foreground"
    )
    with (run_dir / "service.log").open("w", encoding="utf-8") as log_file:
        svc_proc = subprocess.Popen(cmd_str.split(), env=svc_env, stdout=log_file, stderr=log_file)
    token_file = run_dir / "gateway.token"
    for _ in range(150):
        time.sleep(0.0 if token_file.is_file() else 0.1)
    assert token_file.is_file(), (run_dir / "service.log").read_text(encoding="utf-8")[-2000:]
    tok, gw_url = token_file.read_text(encoding="utf-8").strip(), f"http://127.0.0.1:{port}"
    test_env = {**svc_env, "CAPSEM_GATEWAY_URL": gw_url, "VERTEXAI_PROJECT": proj}
    test_env.update({"CAPSEM_GATEWAY_TOKEN": tok, "CAPSEM_VM_PREFIX": prefix})
    test_env["VERTEXAI_LOCATION"] = os.environ.get("VERTEXAI_LOCATION", "global")
    test_env["OPENAI_API_KEY"] = os.environ.get("OPENAI_API_KEY", "local-proxy")
    prev_env = {k: os.environ.get(k) for k in test_env}
    os.environ.update(test_env)
    try:
        yield {"capsem_home": capsem_home, "prefix": prefix, "port": port, "project": proj}
    finally:
        svc_proc.terminate()
        with contextlib.suppress(Exception):
            svc_proc.wait(timeout=10)
        shutil.rmtree(capsem_home, ignore_errors=True)
        for k, v in prev_env.items():
            os.environ.pop(k, None) if v is None else os.environ.__setitem__(k, v)


async def _assert_zero_leaked_vms(prefix: str) -> None:
    hyp = Hypervisor.connect()
    try:
        vms, lbl = (await hyp.list()).sandboxes, "capsem-vm-prefix"
        assert [v.id for v in vms if (v.labels or {}).get(lbl) == prefix] == []
    finally:
        await hyp.close()


def _validate_trajectory_cli(traj_path: Path) -> dict[str, Any]:
    cmd = [sys.executable, "-m", "harbor.utils.trajectory_validator", str(traj_path)]
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    assert proc.returncode == 0, f"trajectory_validator failed: {proc.stdout}\n{proc.stderr}"
    raw, tdir = traj_path.read_text(encoding="utf-8"), traj_path.parent.parent
    assert "truncated_for_transport" not in raw and not list(tdir.glob("*.session.db"))
    return json.loads(raw)


async def _run_harbor_trial(task_dir: Path, trials_dir: Path, **kw: Any) -> tuple[Any, Path]:
    jroot = Path(os.environ.get("HARBOR_LIVE_JOBS_DIR") or trials_dir.parent)
    jdir, model = jroot / trials_dir.name, kw.get("model_name", "vertex_ai/gemini-3.5-flash")
    shutil.rmtree(jdir, ignore_errors=True)
    aname, akw = kw.get("agent_name", AgentName.TERMINUS_2.value), kw.get("agent_kwargs", {})
    acfg = AgentConfig(name=aname, model_name=model, kwargs=akw, env=kw.get("agent_env", {}))
    ekw, hp = {"export_trace": True, "broker_credentials": True}, "harbor_capsem:CapsemEnvironment"
    ecfg = EnvironmentConfig(import_path=hp, override_cpus=2, override_memory_mb=4096, kwargs=ekw)
    tks = [TaskConfig(path=task_dir)]
    jcfg = JobConfig(job_name=jdir.name, jobs_dir=jroot, agents=[acfg], environment=ecfg, tasks=tks)
    job = await Job.create(jcfg)
    return (await job.run()).trial_results[0], next(p for p in jdir.iterdir() if p.is_dir())


async def test_live_three_benchmarks_host_agent(live_capsem_env: Any, tmp_path: Path) -> None:
    res_tb, tdir_tb = await _run_harbor_trial(_find_cached_task(), tmp_path / "tb_fix_git")
    assert res_tb.verifier_result and res_tb.verifier_result.rewards.get("reward") == 1.0
    assert len(list(tdir_tb.glob("*.rows.ndjson.gz"))) == 1 and list(tdir_tb.glob("*.warc.gz"))
    traj_data = _validate_trajectory_cli(tdir_tb / "agent" / "trajectory.json")
    tb_sum = traj_data["extra"]["capsem_summary"]
    assert tb_sum["join_mode"] == "timestamp_window" and tb_sum["exec_events_count"] > 0
    tb_evs = [s.get("extra", {}).get("capsem_events") or {} for s in traj_data["steps"]]
    tb_execs = [e for ev in tb_evs for e in ev.get("exec_events") or ()]
    tb_aud = [a for ev in tb_evs for a in ev.get("audit_events") or ()]
    tb_fs = [f for ev in tb_evs for f in ev.get("fs_events") or ()]
    assert any("git" in str(e.get("stdout_preview")) and not e.get("exit_code") for e in tb_execs)
    assert any("git" in str(a.get("exe")) and a.get("cwd") == "/app/personal-site" for a in tb_aud)
    assert any(".capsem-" in str(f.get("path")) for f in tb_fs)
    for slug, g in (*_EXTRA_BENCH, *_EXTRA_OPT):
        res, out_dir = await _run_harbor_trial(_find_cached_task(g), tmp_path / slug)
        rew = res.verifier_result.rewards.get("reward") if res.verifier_result else None
        assert rew == 1.0 if (slug, g) in _EXTRA_BENCH else rew is not None
        assert len(list(out_dir.glob("*.rows.ndjson.gz"))) == 1 and list(out_dir.glob("*.warc.gz"))
        _validate_trajectory_cli(out_dir / "agent" / "trajectory.json")
    await _assert_zero_leaked_vms(live_capsem_env["prefix"])


@contextlib.asynccontextmanager
async def _started_fix_git_env(tmp_path: Path, name: str) -> AsyncGenerator[CapsemEnvironment]:
    tpaths = TrialPaths(tmp_path / name)
    tpaths.mkdir()
    tcfg = TaskEnvConfig(docker_image="alexgshaw/fix-git:20251031", cpus=2, memory_mb=4096)
    env = CapsemEnvironment(
        _find_cached_task() / "environment", name, f"{name}__s", tpaths, tcfg, export_trace=True
    )
    await env.start(force_build=False)
    try:
        yield env
    finally:
        await env.stop(delete=True)


async def test_live_in_vm_cli_broker_and_turn_id(live_capsem_env: Any, tmp_path: Path) -> None:
    prefix, proj = live_capsem_env["prefix"], live_capsem_env["project"]
    google_tok = _gcloud("auth", "print-access-token")
    ver = f"v1beta1/projects/{proj}/locations/global/publishers/google"
    cli_env = {"GEMINI_API_KEY": google_tok, "GEMINI_API_KEY_AUTH_MECHANISM": "bearer"}
    cli_env.update(GOOGLE_GEMINI_BASE_URL="https://aiplatform.googleapis.com")
    cli_env["GOOGLE_GENAI_API_VERSION"] = ver
    kw = {"agent_name": "gemini-cli", "model_name": "google/gemini-2.5-flash", "agent_env": cli_env}
    res_cli, tdir_cli = await _run_harbor_trial(_find_cached_task(), tmp_path / "cli", **kw)
    assert res_cli.verifier_result and res_cli.verifier_result.rewards.get("reward") == 1.0
    r_files, w_files = list(tdir_cli.glob("*.rows.ndjson.gz")), list(tdir_cli.glob("*.warc.gz"))
    assert len(r_files) == 1 and len(w_files) == 1
    tbls = load_rows_tables(r_files[0], ("model_calls", "tool_calls", "exec_events"))
    mc_rows, tc_count = tbls["model_calls"], len(tbls["tool_calls"])
    exec_cmds = [str(r.get("command") or "") for r in tbls["exec_events"]]
    assert len(mc_rows) >= 1 and mc_rows[0].get("provider") == "google"
    assert "gemini-2.5-flash" in str(mc_rows[0].get("model")) and mc_rows[0].get("turn_id")
    assert int(mc_rows[0].get("input_tokens") or 0) > 0
    assert all(google_tok not in c for c in exec_cmds)
    brokered_refs = [m.group(1) for cmd in exec_cmds if (m := _EXEC_CRED_RE.search(cmd))]
    assert brokered_refs and all(_CRED_REF_RE.match(ref) for ref in brokered_refs)
    enriched = _validate_trajectory_cli(tdir_cli / "agent" / "trajectory.json")
    assert enriched["agent"]["name"] == AgentName.GEMINI_CLI.value
    assert enriched["extra"]["capsem_summary"]["join_mode"] == "turn_id"
    agent_steps = [s for s in enriched["steps"] if s.get("source") == "agent"]
    assert agent_steps
    for s in agent_steps:
        mc_raw = s["extra"]["capsem_model_call"]
        mc = mc_raw[0] if isinstance(mc_raw, list) else mc_raw
        assert "gemini-2.5-flash" in str(mc["model"]) and mc.get("turn_id")
        keys = ("input_tokens", "output_tokens", "duration_ms", "messages_count")
        assert all(int(mc[k] or 0) > 0 for k in keys)
        assert mc.get("request_body_preview") and mc.get("response_body_preview")
    cli_evs = [s["extra"]["capsem_events"] for s in agent_steps]
    cli_nets = [n for ev in cli_evs for n in ev.get("net_events") or ()]
    cli_subs = [sub for ev in cli_evs for sub in ev.get("substitution_events") or ()]
    assert any("aiplatform" in str(n["domain"]) and n["status_code"] == 200 for n in cli_nets)
    assert any(_CRED_REF_RE.match(str(sub.get("substitution_ref") or "")) for sub in cli_subs)
    synth_path = tdir_cli / "agent" / "synthesized_trajectory.json"
    synth = events_to_atif(r_files[0], warc_path=w_files[0], output_path=synth_path)
    _validate_trajectory_cli(synth_path)
    assert [s["source"] for s in synth["steps"][:2]] == ["system", "user"]
    u_phrase = "made some changes to my personal site"
    assert synth["steps"][0]["message"].strip() and u_phrase in synth["steps"][1]["message"]
    s_agents = [s for s in synth["steps"] if s["source"] == "agent"]
    nat_tcs = [tc for s in agent_steps for tc in s.get("tool_calls") or ()]
    syn_tcs = [tc for s in s_agents for tc in s.get("tool_calls") or ()]
    assert len(s_agents) == len(mc_rows) and len(syn_tcs) == tc_count and syn_tcs
    assert [t["function_name"] for t in syn_tcs] == [t["function_name"] for t in nat_tcs]
    assert [t.get("arguments") for t in syn_tcs] == [t.get("arguments") for t in nat_tcs]
    obs_items = [r for s in s_agents for r in (s.get("observation") or {}).get("results") or ()]
    assert [r["source_call_id"] for r in obs_items] == [t["tool_call_id"] for t in syn_tcs]
    fm = synth["final_metrics"]
    assert all(str(r.get("content") or "").strip() for r in obs_items)
    assert all(s["metrics"][k] > 0 for s in s_agents for k in _TOK_KEYS)
    assert fm["total_prompt_tokens"] > 0 and fm["total_completion_tokens"] > 0
    await _assert_zero_leaked_vms(prefix)


async def test_live_local_proxy_and_custom_base_url(live_capsem_env: Any, tmp_path: Path) -> None:
    prefix, proj = live_capsem_env["prefix"], live_capsem_env["project"]
    fix_git_dir = _find_cached_task("terminal-bench/fix-git/*")
    g_tok, proxy_url = _gcloud("auth", "print-access-token"), "http://127.0.0.1:11434/v1"
    p_kw = {"model_name": "openai/google/gemini-3.5-flash", "agent_kwargs": {"api_base": proxy_url}}
    res_p, tdir_p = await _run_harbor_trial(fix_git_dir, tmp_path / "local_proxy", **p_kw)
    assert res_p.verifier_result and res_p.verifier_result.rewards.get("reward") == 1.0
    _validate_trajectory_cli(tdir_p / "agent" / "trajectory.json")
    res_o, tdir_o = await _run_harbor_trial(fix_git_dir, tmp_path / "oracle", agent_name="oracle")
    assert res_o.verifier_result and res_o.verifier_result.rewards.get("reward") == 1.0
    o_traj = _validate_trajectory_cli(tdir_o / "agent" / "trajectory.json")
    o_sum = o_traj["extra"]["capsem_summary"]
    assert o_sum["join_mode"] == "exec_only" and o_sum["fs_events_count"] > 0
    o_evs = [s.get("extra", {}).get("capsem_events") or {} for s in o_traj["steps"]]
    o_aud = [str(a.get("argv") or "") for ev in o_evs for a in ev.get("audit_events") or ()]
    o_execs = [e for ev in o_evs for e in ev.get("exec_events") or ()]
    o_tcs = [tc for s in o_traj["steps"] for tc in s.get("tool_calls") or ()]
    assert any(tc["function_name"] == "bash" and "solve.sh" in str(tc) for tc in o_tcs)
    assert any("solve.sh" in str(e.get("command")) and e.get("exit_code") == 0 for e in o_execs)
    assert all(any(k in a for a in o_aud) for k in (".git/logs/HEAD", "recovery-branch", "master"))
    v_oa = f"https://aiplatform.googleapis.com/v1beta1/projects/{proj}/locations/global/endpoints/openapi"
    a_flags = ("AIDER_SHOW_MODEL_WARNINGS", "AIDER_CHECK_UPDATE", "AIDER_ANALYTICS")
    a_env = {"OPENAI_API_KEY": g_tok, "OPENAI_API_BASE": v_oa, **dict.fromkeys(a_flags, "false")}
    a_kw = {"agent_name": "aider", "model_name": "openai/openai/google/gemini-2.5-flash"}
    _, tdir_a = await _run_harbor_trial(fix_git_dir, tmp_path / "aider", agent_env=a_env, **a_kw)
    a_traj = _validate_trajectory_cli(tdir_a / "agent" / "trajectory.json")
    assert [s["source"] for s in a_traj["steps"][:2]] == ["system", "user"]
    assert "made some changes to my personal site" in a_traj["steps"][1]["message"]
    a_agents = [s for s in a_traj["steps"] if s["source"] == "agent"]
    assert a_agents and all(s["extra"]["capsem_model_call"] for s in a_agents)
    assert all(s["metrics"][k] > 0 for s in a_agents for k in _TOK_KEYS)
    auto_traj = tmp_path / "custom_base_url" / "agent" / "trajectory.json"
    async with _started_fix_git_env(tmp_path, "custom_base_url") as env:
        l_env = {"OPENAI_BASE_URL": proxy_url, "OPENAI_API_KEY": "sk-real-local"}
        l_env["GEMINI_API_KEY"] = "AIzaSyLiveBrokerCheckKey"
        res_local = await env.exec("env | grep -E '^(OPENAI|GEMINI)_API_KEY='", env=l_env)
        out = res_local.stdout or ""
        assert res_local.return_code == 0 and "sk-real-local" not in out and not auto_traj.exists()
        for prefix_key in ("OPENAI_API_KEY=", "GEMINI_API_KEY="):
            line = next(ln for ln in out.splitlines() if ln.startswith(prefix_key))
            assert _CRED_REF_RE.match(line.split("=", 1)[1].strip())
    exec_traj = _validate_trajectory_cli(auto_traj)
    assert auto_traj.is_file() and exec_traj["extra"]["capsem_summary"]["join_mode"] == "exec_only"
    assert any("OPENAI_API_KEY=" in str(s.get("observation")) for s in exec_traj["steps"])
    await _assert_zero_leaked_vms(prefix)
