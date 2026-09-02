use std::time::Duration;

use capsem_proto::mcp_aggregator::{
    read_frame, write_frame, AggregatorClient, AggregatorMethod, AggregatorRequest, AggregatorResponse,
    AggregatorResult,
};
use tokio::io::DuplexStream;

use super::*;

/// A fake aggregator: the driver owns one end of each pipe, the test the other.
struct FakeAggregator {
    /// The subprocess's stdin, as the subprocess reads it.
    stdin: DuplexStream,
    /// The subprocess's stdout, as the subprocess writes it. Dropping it is
    /// the subprocess exiting.
    stdout: Option<DuplexStream>,
    client: AggregatorClient,
    inflight: Inflight,
}

fn spawn_fake() -> FakeAggregator {
    let (client, rx) = AggregatorClient::channel(8);
    let (driver_stdin, fake_stdin) = tokio::io::duplex(64 * 1024);
    let (fake_stdout, driver_stdout) = tokio::io::duplex(64 * 1024);
    let inflight = spawn(rx, driver_stdin, driver_stdout);
    FakeAggregator {
        stdin: fake_stdin,
        stdout: Some(fake_stdout),
        client,
        inflight,
    }
}

impl FakeAggregator {
    async fn next_request(&mut self) -> AggregatorRequest {
        read_frame(&mut self.stdin)
            .await
            .expect("frame from driver")
            .expect("driver stdin still open")
    }

    async fn reply(&mut self, id: u64) {
        let resp = AggregatorResponse {
            id,
            body: AggregatorResult::Ok { ok: true },
        };
        write_frame(self.stdout.as_mut().expect("stdout open"), &resp)
            .await
            .expect("write response");
    }

    fn exit(&mut self) {
        self.stdout.take();
    }
}

const PROMPT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn responses_route_back_to_their_caller() {
    let mut fake = spawn_fake();
    let client = fake.client.clone();
    let call = tokio::spawn(async move { client.request(AggregatorMethod::ListServers).await });

    let req = fake.next_request().await;
    fake.reply(req.id).await;

    let body = tokio::time::timeout(PROMPT, call)
        .await
        .expect("prompt")
        .unwrap()
        .unwrap();
    assert!(matches!(body, AggregatorResult::Ok { ok: true }));
}

// The aggregator can die with requests in flight: a panic, an OOM kill, or
// a session teardown racing a late tool call. Its stdout closes, the driver's
// reader task ends, and every caller parked in the pending map used to wait
// forever because nothing dropped their oneshots. The guest saw each such
// call hang for the full endpoint timeout instead of failing immediately.

#[tokio::test]
async fn subprocess_exit_fails_inflight_callers_immediately() {
    let mut fake = spawn_fake();
    let client = fake.client.clone();
    let call = tokio::spawn(async move { client.request(AggregatorMethod::ListTools).await });

    fake.next_request().await;
    fake.exit();

    let result = tokio::time::timeout(PROMPT, call)
        .await
        .expect("an in-flight call must fail promptly when the aggregator exits")
        .unwrap();
    assert!(result.is_err(), "no aggregator can answer this call: {result:?}");
}

#[tokio::test]
async fn requests_after_subprocess_exit_fail_immediately() {
    let mut fake = spawn_fake();
    fake.exit();
    // Let the reader observe EOF before the next request is issued.
    tokio::task::yield_now().await;

    let result = tokio::time::timeout(PROMPT, fake.client.request(AggregatorMethod::ListPrompts))
        .await
        .expect("a call issued after the aggregator exited must fail promptly");
    assert!(result.is_err(), "{result:?}");
}

// A caller that gives up (the endpoint's per-request timeout) drops its
// receiver, but its sender stayed parked in the pending map until a response
// with that id arrived. A remote MCP server that never answers therefore grew
// the map by one entry per timed-out call for the life of the VM.

#[tokio::test]
async fn abandoned_callers_are_pruned_from_the_pending_map() {
    let mut fake = spawn_fake();
    let client = fake.client.clone();
    let abandoned = tokio::time::timeout(
        Duration::from_millis(20),
        client.request(AggregatorMethod::CallTool {
            name: "slow__tool".into(),
            arguments: serde_json::json!({}),
            timeout_ms: None,
        }),
    )
    .await;
    assert!(abandoned.is_err(), "the caller gave up before any response arrived");
    fake.next_request().await;

    let client = fake.client.clone();
    let live = tokio::spawn(async move { client.request(AggregatorMethod::ListServers).await });
    let req = fake.next_request().await;
    assert_eq!(fake.inflight.count(), 1, "only the live request may remain parked");
    fake.reply(req.id).await;
    tokio::time::timeout(PROMPT, live).await.unwrap().unwrap().unwrap();
}
