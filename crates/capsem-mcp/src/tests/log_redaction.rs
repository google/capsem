//! Tool-call log lines must never carry the guest environment's values.
//!
//! `capsem_create` used to log `?params`, and `CreateParams.env` is where the
//! caller puts `ANTHROPIC_API_KEY=sk-...`; the derived `Debug` wrote that
//! straight into `~/.capsem/run/mcp.log`.

use super::service_boundary::mock_service;
use super::*;
use std::collections::HashMap;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Default)]
struct LogCapture {
    lines: Arc<std::sync::Mutex<Vec<String>>>,
}

struct FieldVisitor<'a>(&'a mut Vec<String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        event.record(&mut FieldVisitor(&mut self.lines.lock().unwrap()));
    }
}

const SECRET: &str = "sk-secret-value-never-logged";

fn secret_env() -> HashMap<String, String> {
    HashMap::from([
        ("ANTHROPIC_API_KEY".to_string(), SECRET.to_string()),
        ("PLAIN".to_string(), "not-a-secret-but-still-a-value".to_string()),
    ])
}

#[tokio::test]
async fn create_log_line_names_env_keys_but_never_values() {
    let capture = LogCapture::default();
    let lines = Arc::clone(&capture.lines);
    let dispatcher = tracing::Dispatch::new(tracing_subscriber::registry().with(capture));

    let service = mock_service(hyper::StatusCode::OK, r#"{"id":"vm-1"}"#).await;
    let handler = CapsemHandler {
        client: Arc::new(UdsClient::new(service.client.uds_path.clone())),
    };
    let params = CreateParams {
        name: Some("box".into()),
        profile: Some("code".into()),
        env: Some(secret_env()),
        ..Default::default()
    };
    handler
        .create(Parameters(params))
        .with_subscriber(dispatcher)
        .await
        .expect("create succeeds against the mock service");

    let joined = lines.lock().unwrap().join("\n");
    assert!(
        joined.contains("capsem_create tool called"),
        "log line missing: {joined}"
    );
    assert!(
        joined.contains("ANTHROPIC_API_KEY"),
        "env keys are operational context: {joined}"
    );
    assert!(!joined.contains(SECRET), "env value leaked into the log: {joined}");
    assert!(
        !joined.contains("not-a-secret-but-still-a-value"),
        "every env value is redacted, not just recognised secrets: {joined}"
    );
}

#[test]
fn params_debug_redacts_every_env_value() {
    let create = CreateParams {
        env: Some(secret_env()),
        ..Default::default()
    };
    let rendered = format!("{create:?}");
    assert!(rendered.contains("ANTHROPIC_API_KEY"), "{rendered}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains("not-a-secret-but-still-a-value"), "{rendered}");

    let run = RunParams {
        command: "env".into(),
        env: Some(secret_env()),
        ..Default::default()
    };
    let rendered = format!("{run:?}");
    assert!(rendered.contains("ANTHROPIC_API_KEY"), "{rendered}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(
        rendered.contains("command: \"env\""),
        "non-secret fields still render: {rendered}"
    );
}

#[test]
fn params_debug_survives_hostile_env_keys() {
    // A key that itself looks like a value must not trick the redaction into
    // printing anything but the key name; values stay hidden regardless.
    let create = CreateParams {
        env: Some(HashMap::from([
            (SECRET.to_string(), "value-for-a-secret-looking-key".to_string()),
            ("\"quoted\" key".to_string(), "v".to_string()),
            (String::new(), "empty-key-value".to_string()),
        ])),
        ..Default::default()
    };
    let rendered = format!("{create:?}");
    assert!(!rendered.contains("value-for-a-secret-looking-key"), "{rendered}");
    assert!(!rendered.contains("empty-key-value"), "{rendered}");
    assert!(
        rendered.contains(SECRET),
        "the key itself is what the operator needs: {rendered}"
    );
}
