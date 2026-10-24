# Exposure and surface control evidence

`[benchmark.routes.preview_controls]` owns finite cycle counts, original request
deadlines, body/token bounds and disposable VM resources. The existing `routes`
collector runs these cases in quick mode too: this is a guest workload, not a
host-only read benchmark. It never places mutations in the concurrent read loop.

The fixture reuses Kingslanding's immutable Redis root with the existing labelled
plain HTTP port responder and `preview_server.py`. One real VM owns declared
surface A and manual preview B. Exact local image admission and the normal
isolated service/gateway owners remain unchanged. No external vendor is called.

POST exposures and DELETE exposure/session use both direct UDS and authenticated
gateway. POST preview-session and surface/session are gateway-owned; their UDS
counterparts are not public operations. The direct phase uses the same real
gateway untimed for capability setup, never counts UDS404 as successful coverage.

Original bootstrap material must name the exact exposure origin and preview port,
remain out of URLs/metric labels and exchange once for the scoped HttpOnly cookie.
The empty303 has no body-byte observation and creates no invented latency sample.
Replay and missing/cross-scope cookie controls use original strict requests;
independent guest hit counts must remain unchanged. Allowed B requests deliver
exact known bytes and increment the workload's request file exactly once.

Revoke must retire exactly one session and close its already admitted original
HTTP keepalive socket, while B remains declared. A fresh original capability can
then reach B. Delete must close that flow, remove B, refuse new minting and preserve
A. The existing transport observes peer EOF/reset on that exact socket: no
reconnect, retry, synthetic EOF or follow-up404 can substitute for retirement.

Surface minting verifies the pinned client's selected first-party script/CSP
contract and exact declared responder bytes. This responder deliberately does not
speak Xpra. It does not certify editor startup, graphical rendering, native input
or paint; those remain explicit state/driver debt. Live effects and independent
guest request counts do not replace canonical physical network/security/archive
joins after producer drain in Ironbank.

Existing raw header/first client body-byte/completion and verified-completion
series, process-tree resource sampler, Rust statistics/store and fresh machine
fitness retain ownership. Source/unit tests use labelled effect doubles and
launch no VM. Source integration is not an executed measurement: the parent must
run a coherent frozen cohort and fit repeated collectors before a performance
claim.
