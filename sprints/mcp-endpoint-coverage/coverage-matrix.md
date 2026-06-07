# MCP + Service Endpoint Coverage Matrix

## Summary

22 MCP tools declared. 12 have real behavioral coverage (55%), 10 have no invocation test at all. 27 of ~43 capsem-service route handlers are **BLIND SPOTS** with zero real test coverage (covering all of `/setup/*`, `/history/*`, `/files/*`, `/mcp/policy`, `/mcp/tools/refresh`, `/mcp/tools/{name}/approve`, and `/service-logs`). Biggest surprise: `capsem_vm_logs` is the most natural "show me what's happening" AI tool yet has zero e2e invocation; and `capsem_stop` is flagged in parity as a drift candidate (MCP-only) while simultaneously being the only tool that tests stop-then-resume persistence. The entire settings/setup/history/files surface (26 handlers) has no integration test hitting the real service binary.

---

## Table 1: MCP Tool Coverage

Source: `crates/capsem-mcp/src/main.rs` (all `#[tool_router]` methods in `CapsemHandler`). Test directory: `tests/capsem-mcp/` (real `capsem-mcp` binary over stdio; `McpSession.call_tool` at `conftest.py:69`).

| Tool | Handler (file:line) | Test(s) | Real coverage? | Notes |
|---|---|---|---|---|
| `capsem_list` | `main.rs:416 fn list` | `test_lifecycle.py::test_create_and_delete`, `test_lifecycle.py::test_list_empty_start`, `test_winterfell_rw.py::test_winterfell_rw`, `test_winterfell_exec.py::test_winterfell_exec`, `test_errors.py::test_two_vms_isolated`, `test_fork_images.py::test_full_lifecycle` | Yes | Asserts `sandboxes` key, specific VM in/out of list |
| `capsem_create` | `main.rs:474 fn create` | `test_lifecycle.py::test_create_and_delete`, `test_lifecycle.py::test_create_with_resources`, `test_lifecycle.py::test_create_auto_name`, `test_lifecycle.py::test_create_duplicate_name`, `test_fork_images.py::test_full_lifecycle`, `test_winterfell_rw.py`, `test_winterfell_exec.py`, `test_winter_is_coming.py` | Yes | Asserts returned ID, resource echo via info, duplicate rejection |
| `capsem_info` | `main.rs:485 fn info` | `test_lifecycle.py::test_info_fields`, `test_lifecycle.py::test_info_nonexistent`, `test_errors.py::test_info_on_deleted_vm`, `test_fork_images.py::test_full_lifecycle`, `test_winter_is_coming.py::test_winter_is_coming` | Yes | Asserts `id`, `status`, `pid`, `size_bytes`, `description`, `forked_from` fields |
| `capsem_exec` | `main.rs:491 fn exec` | `test_exec.py` (9 tests), `test_errors.py::test_exec_on_deleted_vm`, `test_winterfell_exec.py`, `test_fork_images.py`, `test_winter_is_coming.py` | Yes | Asserts stdout content, exit_code, stderr, multi-line output, Linux uname |
| `capsem_read_file` | `main.rs:498 fn read_file` | `test_file_io.py` (8 tests), `test_errors.py::test_read_on_deleted_vm`, `test_winterfell_rw.py`, `test_fork_images.py`, `test_winter_is_coming.py` | Yes | Asserts exact content equality, unicode, multiline, empty, 100KB, overwrite |
| `capsem_write_file` | `main.rs:505 fn write_file` | `test_file_io.py` (write half of 8 tests), `test_errors.py::test_write_on_deleted_vm`, `test_winterfell_rw.py`, `test_fork_images.py`, `test_winter_is_coming.py` | Yes | Asserts `success: true`, content round-trip via read_file |
| `capsem_inspect_schema` | `main.rs:512 fn inspect_schema` | `test_inspect.py::test_schema` | Yes | Asserts `CREATE TABLE` present in output |
| `capsem_inspect` | `main.rs:517 fn inspect` | `test_inspect.py::test_sql_query`, `test_inspect.py::test_bad_sql`, `test_inspect.py::test_inspect_nonexistent_vm` | Yes | Asserts non-empty response, error for bad SQL and missing VM |
| `capsem_delete` | `main.rs:524 fn delete` | `test_lifecycle.py::test_create_and_delete`, `test_lifecycle.py::test_delete_nonexistent`, `test_lifecycle.py::test_delete_twice`, `test_errors.py::*`, `test_fork_images.py::*` | Yes | Asserts VM absent from list, error on double-delete and nonexistent |
| `capsem_stop` | `main.rs:530 fn stop` | `test_winterfell_rw.py::test_winterfell_rw` (line 54), `test_winterfell_exec.py::test_winterfell_exec` (line 56) | Yes | Asserts `success: true`; also tests that list shows Stopped after stop. Note: flagged as drift candidate in `test_cli_parity.py:55` ("MCP-only") |
| `capsem_suspend` | `main.rs:536 fn suspend` | none | No | **NO COVERAGE** -- declared in `test_discovery.py` EXPECTED_TOOLS only; never invoked via `call_tool` |
| `capsem_resume` | `main.rs:542 fn resume` | `test_winterfell_rw.py::test_winterfell_rw` (line 68), `test_winterfell_exec.py::test_winterfell_exec` (line 70) | Yes | Asserts VM name in response; also tests file/data survives resume. Also tests error path (resume after delete) |
| `capsem_persist` | `main.rs:548 fn persist` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:36` MCP_TO_CLI but never invoked in any e2e test |
| `capsem_purge` | `main.rs:555 fn purge` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:37` only |
| `capsem_run` | `main.rs:562 fn run` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:32` only |
| `capsem_fork` | `main.rs:569 fn fork` | `test_fork_images.py` (6 tests), `test_winter_is_coming.py::test_winter_is_coming` | Yes | Asserts name, description, forked_from, size_bytes, error for nonexistent/duplicate source |
| `capsem_vm_logs` | `main.rs:422 fn vm_logs` | none | No | **NO COVERAGE** -- appears only in `test_discovery.py` EXPECTED_TOOLS set (schema check); `call_tool("capsem_vm_logs", ...)` is never called in any test file |
| `capsem_service_logs` | `main.rs:441 fn service_logs` | none | No | **NO COVERAGE** -- MCP-only (no CLI, per `test_cli_parity.py:52`); never invoked in any test |
| `capsem_version` | `main.rs:577 fn version` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:40` and `test_discovery.py` EXPECTED_TOOLS but never invoked |
| `capsem_mcp_servers` | `main.rs:590 fn mcp_servers` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:43` and `test_discovery.py` EXPECTED_TOOLS; never invoked |
| `capsem_mcp_tools` | `main.rs:597 fn mcp_tools` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:44` and `test_discovery.py` EXPECTED_TOOLS; never invoked |
| `capsem_mcp_call` | `main.rs:607 fn mcp_call` | none | No | **NO COVERAGE** -- listed in `test_cli_parity.py:45` and `test_discovery.py` EXPECTED_TOOLS; never invoked |

**Totals:** 22 tools. 12 with real behavioral coverage (55%). 10 with no invocation test (45%).

---

## Table 2: capsem-service Endpoint Coverage

Source: `crates/capsem-service/src/main.rs` routes registered at lines 2671--2715. Real tests: `tests/capsem-service/`, `tests/capsem-lifecycle/`, `tests/capsem-e2e/`, `tests/capsem-mcp/` (via MCP tool calls), `tests/capsem-serial/`. Gateway tests (`tests/capsem-gateway/`) use `MockServiceHandler` and are excluded per sprint scope.

| Method | Path | Handler (file:line) | Test(s) | Notes |
|---|---|---|---|---|
| GET | `/version` | `main.rs:2671` (inline lambda) | none | **BLIND SPOT** -- no test invokes this endpoint against the real service |
| POST | `/provision` | `main.rs:1126 handle_provision` | `test_svc_provision.py::TestProvision`, `test_svc_persistence.py::TestPersistentCreate`, `test_svc_exec_ready.py::TestExecImmediatelyAfterProvision`, `test_vm_lifecycle.py` (all), `test_e2e_lifecycle.py` (via CLI), `test_e2e_mcp.py::TestMcpLifecycle` (via MCP) | Full coverage -- persistent/ephemeral, custom resources, duplicate rejection |
| GET | `/list` | `main.rs:1188 handle_list` | `test_svc_provision.py::TestList`, `test_svc_persistence.py::TestListPersistence`, `test_vm_lifecycle.py` | Yes |
| GET | `/info/{id}` | `main.rs:1247 handle_info` | `test_svc_provision.py::TestInfo`, `test_svc_persistence.py`, `test_vm_lifecycle.py` | Yes |
| GET | `/logs/{id}` | `main.rs:1323 handle_logs` | `test_svc_logs.py::TestLogs`, `test_serial_log.py::TestSerialLog` | Yes -- asserts non-empty logs, boot content, nonexistent VM error |
| POST | `/inspect/{id}` | `main.rs:1887 handle_inspect` | `test_svc_inspect.py::TestInspect` | Yes -- valid SQL, bad SQL, nonexistent VM |
| POST | `/exec/{id}` | `main.rs:1437 handle_exec` | `test_svc_exec.py::TestExec`, `test_svc_exec_ready.py`, `test_vm_lifecycle.py`, `test_e2e_lifecycle.py` | Yes |
| POST | `/write_file/{id}` | `main.rs:1467 handle_write_file` | `test_svc_file_io.py::TestFileIO`, `test_svc_exec_ready.py`, `test_svc_persistence.py::TestResumeLifecycle`, `test_vm_lifecycle.py` | Yes |
| POST | `/read_file/{id}` | `main.rs:1495 handle_read_file` | `test_svc_file_io.py::TestFileIO`, `test_svc_exec_ready.py`, `test_svc_persistence.py::TestResumeLifecycle`, `test_vm_lifecycle.py` | Yes |
| POST | `/stop/{id}` | `main.rs:2178 handle_stop` | `test_svc_persistence.py::TestStopSemantics`, `test_vm_lifecycle.py`, `test_svc_exec_ready.py::TestExecImmediatelyAfterResume` | Yes -- persistent preserves, ephemeral destroys |
| POST | `/suspend/{id}` | `main.rs:2098 handle_suspend` | `test_vm_lifecycle.py::TestSuspendResume` | Yes -- round-trip, rejects ephemeral; also has Rust unit test at `main.rs:3683` |
| DELETE | `/delete/{id}` | `main.rs:2207 handle_delete` | `test_svc_provision.py::TestDelete`, `test_vm_lifecycle.py` | Yes |
| POST | `/resume/{name}` | `main.rs:2259 handle_resume` | `test_svc_persistence.py::TestResumeLifecycle`, `test_svc_exec_ready.py::TestExecImmediatelyAfterResume`, `test_vm_lifecycle.py` | Yes |
| POST | `/persist/{id}` | `main.rs:2272 handle_persist` | `test_svc_persistence.py::TestPersistConvert` | Yes -- converts ephemeral, rejects duplicate name |
| POST | `/purge` | `main.rs:2344 handle_purge` | `test_svc_persistence.py::TestPurge` | Yes -- ephemeral-only and all=true modes |
| POST | `/run` | `main.rs:2413 handle_run` | `test_svc_persistence.py::TestRunEndpoint` | Yes -- output check, exit code propagation |
| GET | `/stats` | `main.rs:1296 handle_stats` | `main.rs:3899 handle_stats_returns_global_data` (Rust unit test only) | **BLIND SPOT** -- no Python integration test hits real service; Rust unit test exists but is in-process |
| GET | `/service-logs` | `main.rs:1355 handle_service_logs` | none | **BLIND SPOT** -- no test of any kind (the `capsem_service_logs` MCP tool bypasses this and reads the log file directly on disk) |
| POST | `/reload-config` | `main.rs:1526 handle_reload_config` | `test_gw_proxy_advanced.py` (gateway mock only -- excluded) | **BLIND SPOT** -- only gateway mock test, no real-service test |
| POST | `/fork/{id}` | `main.rs:1052 handle_fork` | `test_lifecycle_benchmark.py::_run_fork_benchmark` (capsem-serial, not capsem-service/lifecycle) | Smoke -- benchmark calls the endpoint, asserts size_bytes and data survives; no dedicated service-layer test. Also has Rust unit tests at `main.rs:3432--3522` |
| GET | `/settings` | `main.rs:1562 handle_get_settings` | `test_svc_settings.py::TestSettingsTree::test_settings_response_shape` + Rust unit at `main.rs:3955` | Yes |
| POST | `/settings` | `main.rs:1568 handle_save_settings` | `test_svc_settings.py::TestSettingsTree::{test_save_settings_round_trips,test_save_settings_rejects_unknown_key}` + Rust unit at `main.rs:3982` | Yes -- round-trip via GET and unknown-key rejection |
| GET | `/settings/presets` | `main.rs:1587 handle_get_presets` | `test_svc_settings.py::TestPresets::test_presets_lists_medium_and_high` + Rust unit at `main.rs:3966` | Yes |
| POST | `/settings/presets/{id}` | `main.rs:1593 handle_apply_preset` | `test_svc_settings.py::TestPresets::{test_apply_preset_returns_refreshed_tree,test_apply_unknown_preset_rejected}` | Yes |
| POST | `/settings/lint` | `main.rs:1603 handle_lint_config` | `test_svc_settings.py::TestLint::test_lint_returns_array` + Rust unit at `main.rs:3976` | Yes |
| POST | `/settings/validate-key` | `main.rs:1609 handle_validate_key` | `test_svc_settings.py::TestValidateKey::{test_validate_key_unknown_provider_rejected,test_validate_key_empty_key_not_valid,test_validate_key_bogus_anthropic_returns_invalid}` | Yes -- unknown provider, empty key short-circuit, live call against api.anthropic.com |
| GET | `/setup/state` | `main.rs:1623 handle_get_setup_state` | `test_svc_setup.py::TestSetupState::test_state_defaults_when_missing` | Yes -- shape + default values when setup-state.json is absent |
| GET | `/setup/detect` | `main.rs:1642 handle_detect_host_config` | `test_svc_setup.py::TestSetupDetect::test_detect_returns_summary_shape` | Yes -- shape + file-based presence flags false under isolated HOME |
| POST | `/setup/complete` | `main.rs:1655 handle_complete_onboarding` | `test_svc_setup.py::TestSetupState::test_complete_sets_onboarding_flag` | Yes -- flips onboarding_completed and persists to /setup/state |
| GET | `/setup/assets` | `main.rs:1666 handle_asset_status` | `test_svc_setup.py::TestSetupAssets::{test_assets_lists_three_expected_artifacts,test_assets_reports_ready_when_all_present}` | Yes -- shape + ready<->present invariant |
| ~~POST /setup/assets/download~~ | removed | removed in 24633a5 | n/a | **REMOVED** (dead code, no caller) |
| POST | `/setup/corp-config` | `main.rs:1709 handle_corp_config` | `test_svc_setup.py::TestSetupCorpConfig::{test_corp_config_inline_toml,test_corp_config_rejects_invalid_toml,test_corp_config_rejects_empty_payload}` | Yes -- inline TOML round-trip via /settings corp_locked flag, malformed and empty payload rejection |
| GET | `/mcp/servers` | `main.rs:1740 handle_mcp_servers` | `test_svc_mcp_api.py::TestMcpServers::test_servers_returns_list` | Yes -- shape + field types |
| GET | `/mcp/tools` | `main.rs:1775 handle_mcp_tools` | `test_svc_mcp_api.py::TestMcpTools::test_tools_returns_list` | Yes -- returns [] under isolated HOME with no cache |
| GET | `/mcp/policy` | `main.rs:1795 handle_mcp_policy` | `test_svc_mcp_api.py::TestMcpPolicy::test_policy_returns_merged_shape` | Yes -- shape + default_tool_permission defaulting to "allow" |
| POST | `/mcp/tools/refresh` | `main.rs:1819 handle_mcp_refresh` | `test_svc_mcp_api.py::TestMcpToolsRefresh::test_refresh_no_instances_succeeds` | Yes -- instances=0 when no running VMs |
| POST | `/mcp/tools/{name}/approve` | `main.rs:1835 handle_mcp_approve` | `test_svc_mcp_api.py::TestMcpApprove::test_approve_unknown_tool_rejected` | Partial -- 404 path; happy path needs a populated tool cache (requires downstream aggregator) |
| POST | `/mcp/tools/{name}/call` | `main.rs:1854 handle_mcp_call` | `test_svc_mcp_api.py::TestMcpCall::{test_call_without_running_session_rejected,test_call_unknown_tool_with_running_vm_rejected}` | Partial -- 503 no-session path and IPC-plumbing-reaches-aggregator path; downstream happy path tracked as follow-up |
| GET | `/history/{id}` | `main.rs:1944 handle_history` | none | **BLIND SPOT** -- no test of any kind |
| GET | `/history/{id}/processes` | `main.rs:1967 handle_history_processes` | none | **BLIND SPOT** -- no test of any kind |
| GET | `/history/{id}/counts` | `main.rs:1984 handle_history_counts` | none | **BLIND SPOT** -- no test of any kind |
| GET | `/history/{id}/transcript` | `main.rs:2004 handle_history_transcript` | none | **BLIND SPOT** -- no test of any kind |
| GET | `/files/{id}` | `main.rs:902 handle_list_files` | none | **BLIND SPOT** -- no test of any kind |
| GET | `/files/{id}/content` | `main.rs:962 handle_download_file` | none | **BLIND SPOT** -- no test of any kind |
| POST | `/files/{id}/content` | `main.rs:1011 handle_upload_file` | none | **BLIND SPOT** -- no test of any kind |

**Totals:** ~43 route handlers. 16 with real integration test coverage. 27 BLIND SPOTS (including 4 that have Rust in-process unit tests but no Python integration test hitting the real service binary).

---

## Source Issues Found

1. **`capsem_stop` drift candidate is load-bearing.** `test_cli_parity.py:55` flags `capsem_stop` as "MCP-only -- CLI expresses stop via suspend (persistent) or delete (ephemeral). Consider removing." However `capsem_stop` is the only tool that exercises the stop-then-resume persistence path (`test_winterfell_rw.py`, `test_winterfell_exec.py`). Removing it would leave the stop/resume lifecycle untested at the MCP layer.

2. **`capsem_service_logs` reads the log file directly (not via `/service-logs` endpoint).** The MCP tool at `main.rs:441` opens `run_dir.join("service.log")` with `spawn_blocking` + `File::open`. It does NOT call `GET /service-logs` on the service. This means the service endpoint `handle_service_logs` (`main.rs:1355`) and the MCP tool are completely independent code paths. The endpoint has zero real test coverage; the MCP tool also has zero real test coverage.

3. **`/mcp/tools/{name}/call` endpoint exists (`main.rs:1854`) but its MCP-layer counterpart `capsem_mcp_call` (`main.rs:607`) is never invoked in tests.** The MCP gateway integration (the full chain: MCP call -> service -> aggregator -> downstream MCP server) is unproven end-to-end.

4. **`/fork/{id}` has no test in `tests/capsem-service/`.** The only real invocation of `handle_fork` from Python is in `tests/capsem-serial/test_lifecycle_benchmark.py:111`, which is a benchmark (marked `pytest.mark.serial`), not a dedicated correctness test. There are Rust unit tests (`main.rs:3432--3522`) but they are in-process, not real-service.
