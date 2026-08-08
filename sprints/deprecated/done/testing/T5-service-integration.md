# Sprint T5: Service Integration Tests

## Goal

Write Python integration tests that exercise the `capsem-service` HTTP API directly over a Unix domain socket. No MCP layer, no CLI -- raw HTTP requests to validate every endpoint end-to-end with real VMs.

## Files

**Create:**
- `tests/capsem-service/conftest.py` -- fixtures: launch service on temp UDS, HTTP client helper, VM cleanup
- `tests/capsem-service/test_provision.py`
- `tests/capsem-service/test_exec.py`
- `tests/capsem-service/test_file_io.py`
- `tests/capsem-service/test_inspect.py`
- `tests/capsem-service/test_logs.py`

## Tasks

### Fixtures (`conftest.py`)

- [ ] `service` fixture: start `capsem-service` on a temp UDS path, yield client, teardown kills service
- [ ] `http_client` fixture: thin wrapper around `requests_unixsocket` or `httpx` with UDS transport
- [ ] `vm` fixture: provision a default VM, yield its name/ID, delete on teardown
- [ ] Timeout guard: fail test if any single operation exceeds 60 seconds

### POST /provision (`test_provision.py`)

- [ ] Provision with explicit name returns 200 and the name in response
- [ ] Provision without name returns 200 and an auto-generated ID
- [ ] Provision with custom resources (ram, cpus) returns matching values in info
- [ ] Provision with duplicate name returns 409 Conflict
- [ ] Provision at VM limit returns 503 or appropriate capacity error
- [ ] Provision response includes all expected fields (name, id, state, created_at)

### GET /list

- [ ] List with no VMs returns empty array
- [ ] List after provisioning returns array with the VM
- [ ] List includes name, state, and resource info for each VM

### GET /info

- [ ] Info for valid VM returns full details (name, ram, cpus, state, uptime)
- [ ] Info for nonexistent VM returns 404

### POST /exec (`test_exec.py`)

- [ ] Exec `echo hello` returns stdout "hello\n" and exit code 0
- [ ] Exec `sh -c 'echo err >&2'` returns stderr "err\n"
- [ ] Exec `false` returns exit code 1
- [ ] Exec with timeout: long-running command is killed after timeout
- [ ] Exec on nonexistent VM returns 404

### POST /write_file + POST /read_file (`test_file_io.py`)

- [ ] Write then read: content roundtrips exactly
- [ ] Unicode content roundtrips (CJK, emoji, combining characters)
- [ ] Multiline content preserves newlines and trailing newline
- [ ] Empty file write and read back returns empty string
- [ ] 1 MB file write and read back matches (large payload)
- [ ] Read from nonexistent VM returns 404
- [ ] Read nonexistent file path returns error with file path in message

### POST /inspect (`test_inspect.py`)

- [ ] Valid SQL query returns rows (e.g., `SELECT count(*) FROM net_events`)
- [ ] Bad SQL returns error with SQL parse/exec failure details
- [ ] Inspect on nonexistent VM returns 404

### GET /logs (`test_logs.py`)

- [ ] Logs for valid VM returns non-empty string
- [ ] Logs for nonexistent VM returns 404

### DELETE /delete

- [ ] Delete running VM returns 200 and VM is gone from list
- [ ] Delete same VM again returns 404
- [ ] Delete nonexistent VM returns 404

## Verification

- `pytest tests/capsem-service/ -m integration -v` passes all tests
- All tests clean up after themselves (no orphan VMs or sockets)
- Total suite completes in under 5 minutes

## Depends On

- **T0-infrastructure** (test directories, markers, recipes)
- **T1-service-unit-tests** (service code must be stable)
