"""Live Capsem VM acceptance checks for `harbor-capsem-sandbox`."""

from __future__ import annotations

import asyncio
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from capsem import Hypervisor
from capsem.trace import load_rows_tables
from harbor.models.task.config import EnvironmentConfig as TaskEnvConfig
from harbor.models.trial.paths import TrialPaths
from harbor_capsem import CapsemEnvironment

_CRED_REF_RE = re.compile(r"^credential:blake3:[0-9a-f]{64}$")


async def run_live_vm_sandbox_acceptance() -> None:
    """Hermetic VM lifecycle, credential brokering, and ATIF trace export acceptance."""
    hyp = Hypervisor.connect()
    try:
        with tempfile.TemporaryDirectory(prefix="harbor-capsem-live-") as tmp_raw:
            tmp_dir = Path(tmp_raw)
            env_dir = tmp_dir / "environment"
            env_dir.mkdir(parents=True, exist_ok=True)
            (env_dir / "Dockerfile").write_text("FROM scratch\n", encoding="utf-8")
            trial_paths = TrialPaths(trial_dir=tmp_dir / "trial")
            trial_paths.mkdir()
            env = CapsemEnvironment(
                environment_dir=env_dir,
                environment_name="gate_harbor_vm_acceptance",
                session_id="gate_harbor_vm_acceptance__1",
                trial_paths=trial_paths,
                task_env_config=TaskEnvConfig(cpus=2, memory_mb=2048),
                execution_mode="vm",
                export_trace=True,
                broker_credentials=True,
            )
            vm_id = ""
            try:
                await env.start(force_build=False)
                assert env.vm_id is not None
                vm_id = env.vm_id

                exec_res = await env.exec("echo HARBOR_CAPSEM_VM_ACCEPTANCE_OK")
                assert exec_res.return_code == 0, exec_res.stderr
                assert (exec_res.stdout or "").strip() == "HARBOR_CAPSEM_VM_ACCEPTANCE_OK"

                secret = "AIzaSyLiveBrokerCheckKey"
                cred_res = await env.exec(
                    "env | grep '^GEMINI_API_KEY='",
                    env={"GEMINI_API_KEY": secret},
                )
                cred_out = (cred_res.stdout or "").strip()
                assert cred_res.return_code == 0, cred_res.stderr
                assert secret not in cred_out
                ref_val = cred_out.split("=", 1)[1].strip()
                assert _CRED_REF_RE.match(ref_val), cred_out

                src_file = tmp_dir / "upload.txt"
                dst_file = tmp_dir / "download.txt"
                src_file.write_text("harbor-capsem-roundtrip\n", encoding="utf-8")
                await env.upload_file(src_file, "/workspace/roundtrip.txt")
                await env.download_file("/workspace/roundtrip.txt", dst_file)
                assert dst_file.read_text(encoding="utf-8") == "harbor-capsem-roundtrip\n"
            finally:
                await env.stop(delete=True)

            try:
                await env.stop(delete=False)
            except NotImplementedError:
                pass
            else:
                raise AssertionError("expected NotImplementedError on env.stop(delete=False)")

            assert vm_id not in {m.id for m in (await hyp.list()).sandboxes}, (
                f"env.stop(delete=True) leaked VM {vm_id}"
            )
            rows_files = list(trial_paths.trial_dir.glob("*.rows.ndjson.gz"))
            warc_files = list(trial_paths.trial_dir.glob("*.warc.gz"))
            assert len(rows_files) == 1 and len(warc_files) == 1
            assert not list(trial_paths.trial_dir.glob("*.session.db"))
            tables = load_rows_tables(rows_files[0], ("exec_events",))
            exec_cmds = [str(r.get("command") or "") for r in tables["exec_events"]]
            assert any("HARBOR_CAPSEM_VM_ACCEPTANCE_OK" in cmd for cmd in exec_cmds)
            assert all("AIzaSyLiveBrokerCheckKey" not in cmd for cmd in exec_cmds)

            traj_path = trial_paths.agent_dir / "trajectory.json"
            assert traj_path.is_file(), f"missing exported trajectory at {traj_path}"
            proc = subprocess.run(
                [sys.executable, "-m", "harbor.utils.trajectory_validator", str(traj_path)],
                capture_output=True,
                text=True,
                check=False,
            )
            assert proc.returncode == 0, (
                f"trajectory_validator failed: {proc.stdout}\n{proc.stderr}"
            )
            raw_traj = traj_path.read_text(encoding="utf-8")
            assert "truncated_for_transport" not in raw_traj
            traj = json.loads(raw_traj)
            assert traj["schema_version"] == "ATIF-v1.7"
            assert traj["extra"]["capsem_summary"]["join_mode"] == "exec_only"
            print("HARBOR_CAPSEM_VM_ACCEPTANCE_OK")
    finally:
        await hyp.close()


async def main() -> None:
    await run_live_vm_sandbox_acceptance()


if __name__ == "__main__":
    asyncio.run(main())
