# Gateway specification

`openapi.json` is generated from the Rust wire contracts in `capsem-api` and
served by the gateway at `GET /openapi.json`, using normal bearer authentication.
The document covers lifecycle, execution, files, private networks, profile
listing, update, managed restart and statistics operations. Further SDK operations
join the same document as their gateway contracts are typed.

Do not edit the JSON by hand. From the repository root, regenerate it with:

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 120 -- \
  cargo run --quiet -p capsem-api --example export_openapi > sdk/specification/openapi.json
```

The contract tests compare the checked-in document with the Rust export and
verify that every schema reference resolves. Gateway tests check authentication
and that every documented method/path reaches the real proxy route table.

SDKs supply their own gateway URL and bearer token. They never use the
service's UDS path, even when an existing response includes that diagnostic
field. Container creation and port exposure join this contract only when their
service-owned lifecycle and security admission are implemented.
